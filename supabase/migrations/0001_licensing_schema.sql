-- Diskwise licensing schema.
--
-- Every table here is written by exactly one actor: the service role, from an Edge Function. The
-- browser and the desktop app never write directly. Row Level Security is enabled on every table
-- and there is no policy that grants INSERT, UPDATE or DELETE to `anon` or `authenticated`.
--
-- Conventions:
--   * money is stored as an integer in the smallest unit of `currency` (paise for INR)
--   * timestamps are bigint epoch milliseconds, so they are comparable across clients
--   * provider identifiers are unique-constrained, so a replayed webhook cannot create a second row
--   * a revoked licence is *kept*, never deleted: the audit trail is the product

create extension if not exists pgcrypto;

-- ---------------------------------------------------------------- products --

-- The thing being sold. `id` must equal the Rust `CommercialConfig::product_id`.
create table if not exists products (
  id            text primary key,
  name          text        not null,
  slug          text        not null unique,
  active        boolean     not null default true,
  created_at    bigint      not null
);

-- ------------------------------------------------- entitlement configuration --

-- One row per sellable tier. This is the server-side twin of the Rust `CommercialConfig`, and it is
-- the authority on price: a checkout is priced from here, never from anything the client sends.
create table if not exists entitlement_configs (
  id                          text    primary key,
  product_id                  text    not null references products (id) on delete restrict,
  name                        text    not null,
  price_minor_units           bigint  not null check (price_minor_units >= 0),
  currency                    text    not null check (currency ~ '^[A-Z]{3}$'),
  -- true = one-time purchase; the entitlement itself never expires.
  permanent                   boolean not null,
  -- months of update eligibility, or null for the life of the product.
  update_eligibility_months   integer check (update_eligibility_months is null or update_eligibility_months > 0),
  device_limit                integer not null check (device_limit between 1 and 100),
  offline_lease_days          integer not null check (offline_lease_days between 1 and 365),
  active                      boolean not null default true,
  created_at                  bigint  not null,
  updated_at                  bigint  not null,
  unique (product_id, id)
);

-- ------------------------------------------------------------------- orders --

-- One row per payment, keyed by the provider's own payment id. The unique constraint on
-- `provider_payment_id` is what makes duplicate webhook delivery harmless: the second insert
-- conflicts and is treated as "already processed".
create table if not exists orders (
  id                    uuid    primary key default gen_random_uuid(),
  provider              text    not null default 'dodo' check (provider in ('dodo', 'manual')),
  provider_payment_id   text    not null unique,
  provider_session_id   text,
  provider_checkout_id  text,
  product_id            text    not null references products (id) on delete restrict,
  tier_id               text    not null,
  amount_minor_units    bigint  not null check (amount_minor_units >= 0),
  currency              text    not null check (currency ~ '^[A-Z]{3}$'),
  -- 'succeeded' | 'failed' | 'processing' | 'cancelled' | 'refunded' | 'disputed'
  status                text    not null,
  customer_email        text    not null,
  customer_name         text,
  customer_id           text,
  metadata              jsonb   not null default '{}'::jsonb,
  -- The environment the payment was taken in. A test order can never become a live entitlement.
  environment           text    not null check (environment in ('test', 'live')),
  paid_at               bigint,
  created_at            bigint  not null,
  updated_at            bigint  not null
);

create index if not exists orders_customer_email_idx on orders (customer_email);
create index if not exists orders_status_idx on orders (status);
-- Recovery by order reference is a supported path, so it needs to be fast and it needs to be
-- unique per customer rather than globally.
create index if not exists orders_provider_session_idx on orders (provider_session_id);

-- ----------------------------------------------------------------- licences --

-- A licence is issued for exactly one paid order. `key_hash` is a SHA-256 of the licence key:
-- enough to look a key up, and not enough to reconstruct it if the database leaks.
--
-- The key itself is shown to the customer once, at issuance, and is recoverable only through the
-- documented recovery flow. It is never written here in plaintext.
create table if not exists licences (
  id                  uuid    primary key default gen_random_uuid(),
  order_id            uuid    not null references orders (id) on delete restrict,
  product_id          text    not null references products (id) on delete restrict,
  tier_id             text    not null,
  -- First 8 characters of the key, for display and support. Not enough to authenticate.
  key_prefix          text    not null,
  -- SHA-256 hex of the full licence key. The lookup path.
  key_hash            text    not null unique,
  -- Which signing key signed this licence. Enables rotation without invalidating old licences.
  signing_key_id      text    not null,
  -- The signed payload schema version, so an old licence keeps a known shape.
  schema_version      integer not null check (schema_version > 0),
  permanent           boolean not null,
  lease_days          integer not null check (lease_days > 0),
  device_limit        integer not null check (device_limit between 1 and 100),
  update_until        bigint,
  issued_at           bigint  not null,
  -- 'active' | 'revoked' | 'refunded'
  status              text    not null default 'active',
  status_reason       text,
  status_changed_at   bigint,
  created_at          bigint  not null,
  updated_at          bigint  not null,
  constraint licences_status_check check (status in ('active', 'revoked', 'refunded'))
);

-- One licence per order. A duplicate webhook cannot mint a second licence for the same payment.
create unique index if not exists licences_order_unique on licences (order_id);
create index if not exists licences_status_idx on licences (status);

-- ---------------------------------------------------------- device activations --

-- The authoritative device ledger. The desktop app keeps a local mirror so it can show the same
-- list offline, but this table is what enforces the limit.
--
-- The partial unique index is the atomic enforcement point: two concurrent activations on a
-- licence that already has `device_limit` active devices cannot both insert, because the second
-- one's row would violate the unique constraint once the count is checked in the same transaction.
-- `activate_device` performs that check with `FOR UPDATE` on the parent licence row, so the count
-- and the insert are serialised.
create table if not exists device_activations (
  id                uuid    primary key default gen_random_uuid(),
  licence_id        uuid    not null references licences (id) on delete cascade,
  device_binding    text    not null,
  device_name       text    not null,
  device_platform   text    not null default 'windows',
  activated_at      bigint  not null,
  last_seen_at      bigint  not null,
  deactivated_at    bigint,
  deactivate_reason text,
  constraint device_activations_binding_shape check (device_binding ~ '^dev-[0-9a-f]{8,64}$')
);

-- One active activation per binding per licence. Re-activating the same machine refreshes the row
-- instead of consuming a second slot, which is what makes a reinstall free.
create unique index if not exists device_activations_active_unique
  on device_activations (licence_id, device_binding)
  where deactivated_at is null;

create index if not exists device_activations_licence_idx on device_activations (licence_id);

-- ------------------------------------------------------------ webhook idempotency --

-- Every webhook Dodo delivers, keyed by its `webhook-id`. Dodo retries until it gets a 2xx, so
-- this table is what turns "at least once" into "exactly once".
create table if not exists webhook_events (
  webhook_id     text    primary key,
  event_type     text    not null,
  -- SHA-256 of the raw body, so a replayed id carrying a different body is caught.
  body_hash      text    not null,
  received_at    bigint  not null,
  processed_at   bigint,
  attempts       integer not null default 0 check (attempts >= 0),
  last_error     text,
  constraint webhook_events_type_shape check (event_type ~ '^[a-z][a-z0-9_.]*$')
);

-- -------------------------------------------------------------------- refunds --

create table if not exists refunds (
  id                    uuid    primary key default gen_random_uuid(),
  order_id              uuid    not null references orders (id) on delete restrict,
  provider_refund_id    text    not null unique,
  amount_minor_units    bigint  not null check (amount_minor_units >= 0),
  currency              text    not null check (currency ~ '^[A-Z]{3}$'),
  reason                text,
  -- 'succeeded' | 'failed'
  status                text    not null,
  created_at            bigint  not null
);

-- ---------------------------------------------------------------- revocations --

-- Every revocation, including refund-driven ones, so the reason survives even if the licence row is
-- later re-issued.
create table if not exists revocations (
  id           uuid    primary key default gen_random_uuid(),
  licence_id   uuid    not null references licences (id) on delete cascade,
  reason       text    not null,
  source       text    not null check (source in ('refund', 'chargeback', 'dispute', 'support', 'admin')),
  actor        text    not null,
  created_at   bigint  not null
);

-- ------------------------------------------------------------------- audit --

-- A plain append-only log. It deliberately stores no file paths, no licence keys and no secrets:
-- only ids, event names and short human notes.
create table if not exists audit_log (
  id           bigserial primary key,
  at_ms        bigint  not null,
  actor        text    not null,
  action       text    not null,
  licence_id   uuid,
  order_id     uuid,
  detail       text    not null default ''
);

create index if not exists audit_log_at_idx on audit_log (at_ms desc);

-- ------------------------------------------------------- licence recovery --

-- A one-time, short-lived token that lets a customer who lost their key recover a licence by
-- proving control of the email address on the order. The token is stored hashed, so a database
-- leak does not hand out working recovery links.
create table if not exists recovery_tokens (
  id            uuid    primary key default gen_random_uuid(),
  token_hash    text    not null unique,
  order_id      uuid    not null references orders (id) on delete cascade,
  email         text    not null,
  expires_at    bigint  not null,
  consumed_at   bigint,
  attempts      integer not null default 0 check (attempts >= 0),
  created_at    bigint  not null,
  constraint recovery_tokens_attempts_cap check (attempts <= 10)
);

-- ------------------------------------------------------------- rate limiting --

-- A simple counter per (bucket, subject, window). Edge Functions increment it before doing any
-- paid or privileged work and refuse when the count exceeds the cap for the window.
create table if not exists rate_limits (
  bucket        text    not null,
  subject       text    not null,
  window_start  bigint  not null,
  count         integer not null default 0 check (count >= 0),
  primary key (bucket, subject, window_start)
);
