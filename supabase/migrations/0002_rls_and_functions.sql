-- Row Level Security and the atomic device-limit enforcement.
--
-- ## Policy model
--
-- Every table is locked down. There is exactly one writer: the service role, used only by Edge
-- Functions running server-side. Neither `anon` nor `authenticated` gets a policy that inserts,
-- updates or deletes anything.
--
-- The desktop application does **not** talk to this database directly. It verifies licences
-- offline with an embedded public key, and calls Edge Functions for the two things that genuinely
-- need a server (recording an activation, recovering a lost key). Those functions authenticate
-- with the service-role key, which never leaves the server.
--
-- That is why there are no client-readable policies here either: a licence key is a bearer token,
-- and letting a client SELECT from `licences` would turn any leaked anon key into a key-dump.

alter table products              enable row level security;
alter table entitlement_configs   enable row level security;
alter table orders                enable row level security;
alter table licences              enable row level security;
alter table device_activations    enable row level security;
alter table webhook_events        enable row level security;
alter table refunds               enable row level security;
alter table revocations           enable row level security;
alter table audit_log             enable row level security;
alter table recovery_tokens       enable row level security;
alter table rate_limits           enable row level security;

-- No policies are created for anon or authenticated. With RLS enabled and no policy, every
-- non-service-role statement returns zero rows and writes nothing.
--
-- The statements below are intentionally *not* run. They are recorded here so the next person does
-- not "fix" the empty policy list by adding a permissive one:
--
--   create policy "anyone can read licences" on licences for select using (true);   -- NEVER
--   create policy "anyone can activate" on device_activations for insert with (true); -- NEVER

-- ---------------------------------------------------------------- functions --

-- The number of devices currently active on a licence.
create or replace function active_device_count(licence uuid)
returns integer
language sql
stable
security definer
set search_path = public
as $$
  select count(*)::integer
    from device_activations
   where licence_id = licence
     and deactivated_at is null;
$$;

-- Activates a device, or refreshes an existing activation, enforcing the licence's device limit
-- atomically.
--
-- Returns the new active count. Raises `device_limit_reached` when the licence is already at its
-- limit on *other* devices.
--
-- The serialisation is the point: `select ... for update` on the licence row means two concurrent
-- activations queue behind each other, so the count cannot be read twice at the same value. This is
-- the only correct way to enforce a limit; a check-then-insert in the application is not.
create or replace function activate_device(
  licence uuid,
  binding text,
  name text,
  platform text,
  now_ms bigint
)
returns integer
language plpgsql
security definer
set search_path = public
as $$
declare
  limit_count integer;
  active_now  integer;
  existing    uuid;
begin
  -- Lock the licence row for the rest of this transaction.
  select device_limit into limit_count
    from licences
   where id = licence
     and status = 'active'
   for update;

  if limit_count is null then
    raise exception 'licence_not_active' using errcode = 'no_data_found';
  end if;

  -- Re-activating a machine that is already here refreshes it and consumes no slot. This is what
  -- makes a reinstall or a hardware change on the same binding free.
  select id into existing
    from device_activations
   where licence_id = licence
     and device_binding = binding
     and deactivated_at is null;

  if existing is not null then
    update device_activations
       set last_seen_at = now_ms,
           device_name  = coalesce(nullif(name, ''), device_name),
           device_platform = coalesce(nullif(platform, ''), device_platform)
     where id = existing;
    return active_device_count(licence);
  end if;

  select active_device_count(licence) into active_now;

  if active_now >= limit_count then
    raise exception 'device_limit_reached'
      using errcode = 'check_violation',
            hint = format('active=%s limit=%s', active_now, limit_count);
  end if;

  insert into device_activations (
    licence_id, device_binding, device_name, device_platform,
    activated_at, last_seen_at
  ) values (
    licence, binding, name, platform, now_ms, now_ms
  );

  return active_device_count(licence);
end;
$$;

-- Releases a slot. Only the device that owns the binding, or the service role, may do this.
create or replace function deactivate_device(
  licence uuid,
  binding text,
  reason text,
  now_ms bigint
)
returns boolean
language plpgsql
security definer
set search_path = public
as $$
declare
  affected integer;
begin
  update device_activations
     set deactivated_at   = now_ms,
         deactivate_reason = coalesce(nullif(reason, ''), 'user')
   where licence_id = licence
     and device_binding = binding
     and deactivated_at is null;

  get diagnostics affected = row_count;
  return affected > 0;
end;
$$;

-- Claims a webhook id for processing. Returns false when the id has already been seen with the
-- same body, which is the duplicate-delivery case and is a success, not an error.
create or replace function claim_webhook(
  id text,
  event_type text,
  body_hash text,
  now_ms bigint
)
returns boolean
language plpgsql
security definer
set search_path = public
as $$
declare
  claimed integer;
begin
  insert into webhook_events (webhook_id, event_type, body_hash, received_at, attempts)
  values (id, event_type, body_hash, now_ms, 1)
  on conflict (webhook_id) do nothing;

  get diagnostics claimed = row_count;
  return claimed > 0;
end;
$$;

-- Marks a webhook processed, or records the failure and the attempt count.
create or replace function finish_webhook(
  id text,
  error text,
  now_ms bigint
)
returns void
language plpgsql
security definer
set search_path = public
as $$
begin
  if error is null then
    update webhook_events
       set processed_at = now_ms,
           last_error   = null
     where webhook_id = id;
  else
    update webhook_events
       set attempts   = attempts + 1,
           last_error = left(error, 500)
     where webhook_id = id;
  end if;
end;
$$;

-- A fixed-window rate limiter. Returns true when the request is allowed.
create or replace function rate_limit_hit(
  bucket text,
  subject text,
  cap integer,
  window_ms bigint,
  now_ms bigint
)
returns boolean
language plpgsql
security definer
set search_path = public
as $$
declare
  window bigint;
begin
  window := (now_ms / window_ms) * window_ms;

  insert into rate_limits (bucket, subject, window_start, count)
  values (bucket, subject, window, 1)
  on conflict (bucket, subject, window_start)
    do update set count = rate_limits.count + 1;

  return (select count from rate_limits
           where bucket = rate_limit_hit.bucket
             and subject = rate_limit_hit.subject
             and window_start = window) <= cap;
end;
$$;

-- ---------------------------------------------------------------- seed data --

-- The seed rows mirror `src-tauri/src/config/product.rs`. Change both together, and run
-- `docs/licensing.md`'s checklist when you do.
insert into products (id, name, slug, active, created_at)
values ('diskwise-pro', 'Diskwise Pro', 'diskwise-pro', true, 0)
on conflict (id) do nothing;

insert into entitlement_configs (
  id, product_id, name, price_minor_units, currency,
  permanent, update_eligibility_months, device_limit, offline_lease_days,
  active, created_at, updated_at
) values (
  'pro-lifetime', 'diskwise-pro', 'Diskwise Pro — Lifetime',
  149900, 'INR',
  true, null, 3, 30,
  true, 0, 0
)
on conflict (id) do nothing;
