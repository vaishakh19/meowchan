/**
 * Entitlements and feature gating.
 *
 * Rules of the road (see docs/ARCHITECTURE.md#licensing):
 *
 *  1. The **backend** is the authority. The Rust `licensing` module verifies the license
 *     signature (Ed25519) and the expiry clock, then derives the `FeatureFlags` that ship with
 *     every `Entitlement` response. The frontend never decides that a feature is unlocked.
 *  2. The frontend uses this module to *present* gating (badges, upgrade prompts, disabled
 *     controls) and to run the browser-preview transport, which has no backend to ask.
 *  3. A failed or missing license check must never block scanning: `coreAnalysis` is always
 *     true and every gate degrades to "free tier", never to "broken".
 */

import type { Entitlement, EntitlementState, FeatureFlags, PlanTier } from '@/types/contracts';

/** Duplicate groups the free tier will report. Generous: the primary feature stays useful. */
export const FREE_DUPLICATE_GROUP_LIMIT = 25;

/** Length of the Pro trial offered on first launch, in days. */
export const TRIAL_DAYS = 14;

const FREE_FEATURES: FeatureFlags = {
  coreAnalysis: true,
  scanHistory: false,
  advancedDuplicates: false,
  customCleanupRules: false,
  scheduledScans: false,
  exportReports: false,
  advancedFilters: false,
};

const PRO_FEATURES: FeatureFlags = {
  coreAnalysis: true,
  scanHistory: true,
  advancedDuplicates: true,
  customCleanupRules: true,
  scheduledScans: true,
  exportReports: true,
  advancedFilters: true,
};

const DAY_MS = 86_400_000;

/** Feature set for an entitlement state. Pure and total — every state maps to a full flag set. */
export function featuresFor(state: EntitlementState): FeatureFlags {
  return state === 'pro-active' || state === 'trial' ? { ...PRO_FEATURES } : { ...FREE_FEATURES };
}

/** True when the state grants Pro capabilities right now. */
export function isProActive(state: EntitlementState): boolean {
  return state === 'pro-active' || state === 'trial';
}

/** Free entitlement, the default for a fresh install. */
export function freeEntitlement(now = Date.now()): Entitlement {
  return {
    state: 'free',
    tier: 'free',
    features: featuresFor('free'),
    licenseKeyMasked: null,
    permanent: false,
    leaseUntil: null,
    licenseId: null,
    verifiedAt: now,
    source: 'default',
    reason: null,
    daysRemaining: null,
    deviceLimit: 1,
    activeDevices: 0,
    duplicateGroupLimit: FREE_DUPLICATE_GROUP_LIMIT,
  };
}

/** `DWPRO-ABCD-...-WXYZ` → `DWPRO-****-WXYZ` */
/** One-line summary of the current entitlement, for the licensing screen. */
export function entitlementHeadline(entitlement: Entitlement): string {
  switch (entitlement.state) {
    case 'free':
      return 'Free plan — every core feature, no account, no payment.';
    case 'trial':
      return `Pro trial — ${entitlement.daysRemaining ?? 0} day(s) remaining.`;
    case 'pro-active':
      return 'Pro is active. Thank you for supporting Diskwise.';
    case 'pro-expired':
      return 'Pro needs re-verifying. Diskwise continues on the free tier; scanning was never gated.';
    case 'invalid':
    default:
      return 'No valid licence. Scanning continues on the free tier.';
  }
}

/** True when the entitlement unlocks paid features (trial or active pro). */
export function isPaid(entitlement: Entitlement): boolean {
  return isProActive(entitlement.state);
}

export function maskLicenseKey(key: string): string {
  const trimmed = key.trim();
  if (trimmed.length <= 8) return '*'.repeat(trimmed.length);
  return `${trimmed.slice(0, 6)}${'*'.repeat(Math.max(4, trimmed.length - 10))}${trimmed.slice(-4)}`;
}

/**
 * Days left on the offline validation lease.
 *
 * `null` means the lease is perpetual, which is the normal case for a one-time purchase. This is
 * deliberately *not* a countdown to losing the product.
 */
export function daysRemaining(leaseUntil: number | null, now = Date.now()): number | null {
  if (leaseUntil === null) return null;
  const ms = leaseUntil - now;
  if (ms <= 0) return 0;
  return Math.ceil(ms / DAY_MS);
}

/**
 * Resolves the entitlement implied by a *verified* license payload.
 * Signature verification happens in Rust; this function only applies the clock rules so the UI
 * and the backend agree on the state machine.
 */
export interface VerifiedLicensePayload {
  plan: PlanTier;
  /**
   * `true` for a one-time purchase. A permanent licence never expires, whatever the lease says.
   */
  permanent: boolean;
  /** Epoch ms at which the offline validation lease ends, or `null` for a perpetual lease. */
  leaseUntil: number | null;
  licenseId: string;
  /** How many devices the licence may be active on at once. */
  deviceLimit?: number;
  /** How many devices are currently active. */
  activeDevices?: number;
}

export function entitlementFromVerifiedLicense(
  payload: VerifiedLicensePayload,
  now = Date.now(),
): Entitlement {
  // A lapsed *lease* on a permanent licence is a re-verification problem, not a loss of ownership.
  // A lapsed lease on a non-permanent licence is a real expiry.
  const leaseLapsed = payload.leaseUntil !== null && payload.leaseUntil <= now;
  const expired = leaseLapsed && !payload.permanent;
  const state: EntitlementState = expired
    ? 'pro-expired'
    : payload.plan === 'trial'
      ? 'trial'
      : 'pro-active';
  return {
    state,
    tier: expired ? 'free' : payload.plan,
    features: featuresFor(state),
    licenseKeyMasked: null,
    permanent: payload.permanent,
    leaseUntil: payload.leaseUntil,
    licenseId: payload.licenseId,
    verifiedAt: now,
    source: 'license-key',
    reason: expired
      ? 'This licence needs to be re-verified. Connect to the internet and re-activate; your purchase is not affected.'
      : null,
    daysRemaining: daysRemaining(payload.leaseUntil, now),
    deviceLimit: payload.deviceLimit ?? 1,
    activeDevices: payload.activeDevices ?? 0,
    duplicateGroupLimit: 0,
  };
}

/**
 * Applies the offline grace period: a Pro license that cannot be re-verified keeps working for
 * `graceDays` so a laptop that is offline for a week is not locked out.
 * Returns the state the entitlement should degrade to once the grace period ends.
 */
export function applyOfflineGrace(
  entitlement: Entitlement,
  now = Date.now(),
  graceDays = 7,
): Entitlement {
  if (entitlement.state !== 'pro-active' && entitlement.state !== 'trial') return entitlement;
  // A perpetual lease never lapses, so a permanent licence is never degraded by being offline.
  if (entitlement.leaseUntil === null) return entitlement;
  // A permanent licence with a lapsed lease stays paid: only the re-validation window ran out.
  if (entitlement.permanent) return entitlement;
  const graceUntil = entitlement.leaseUntil + graceDays * DAY_MS;
  if (now <= graceUntil) return entitlement;
  return { ...entitlement, state: 'pro-expired', tier: 'free', features: featuresFor('pro-expired') };
}

/** The transition table, used by the UI copy and by tests. */
export const ENTITLEMENT_TRANSITIONS: Record<EntitlementState, EntitlementState[]> = {
  free: ['trial', 'pro-active', 'invalid'],
  trial: ['pro-active', 'pro-expired', 'invalid'],
  'pro-active': ['pro-expired', 'invalid'],
  'pro-expired': ['free', 'pro-active', 'invalid'],
  invalid: ['free', 'pro-active'],
};

export function isValidTransition(from: EntitlementState, to: EntitlementState): boolean {
  if (from === to) return true;
  return ENTITLEMENT_TRANSITIONS[from].includes(to);
}

export interface GateCheck {
  allowed: boolean;
  /** Shown next to the disabled control. */
  reason: string | null;
}

/** Single choke point the UI calls before enabling a paid action. */
export function checkFeature(
  entitlement: Entitlement,
  feature: keyof FeatureFlags,
): GateCheck {
  if (entitlement.features[feature]) return { allowed: true, reason: null };
  return {
    allowed: false,
    reason: 'Diskwise Pro',
  };
}

/**
 * Applies the free-tier duplicate group limit.
 * `0` means unlimited. Returns the truncated list plus a flag so the UI can offer an upgrade.
 */
export function applyDuplicateLimit<T>(groups: T[], limit: number): { groups: T[]; limited: boolean } {
  if (limit <= 0 || groups.length <= limit) return { groups, limited: false };
  return { groups: groups.slice(0, limit), limited: true };
}
