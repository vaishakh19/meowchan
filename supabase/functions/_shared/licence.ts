// Licence payload construction and Ed25519 signing, server-side only.
//
// The private key lives here and nowhere else. The desktop application carries only the matching
// public key, so a leaked client bundle cannot mint a licence.
//
// ## Wire format
//
// A licence key is `<base64url(payload JSON)>.<base64url(signature)>`. The payload is exactly the
// JSON the desktop app parses, in camelCase, with these fields:
//
//   schemaVersion  payload schema this licence was minted with
//   keyId          which signing key signed it, so rotation is distinguishable from revocation
//   licenseId      stable identifier, never the bearer key itself
//   productId      must equal the desktop build's `product_id`
//   tierId         must be a tier the desktop build knows
//   issuedAt       epoch milliseconds
//   permanent      true for a one-time purchase: the entitlement itself never expires
//   leaseUntil     when the offline validation lease ends; null when the tier grants no lease
//   deviceBinding  the machine this licence was activated for
//   deviceLimit    how many machines may be active at once
//   metadata       free-form, never contains personal data
//
// ## Signing
//
// Ed25519 over the exact bytes of the base64url-decoded payload. Node's `sign` with a `null`
// digest is the Ed25519 path; passing SHA-256 there would produce an unusable signature. The Rust
// side verifies the same bytes with `ed25519-dalek`.

import { createPrivateKey, createPublicKey, sign, verify } from 'node:crypto';

export interface LicensePayloadInput {
  licenseId: string;
  productId: string;
  tierId: string;
  issuedAt: number;
  permanent: boolean;
  /** Epoch milliseconds, or null when the tier grants no offline lease. */
  leaseUntil: number | null;
  deviceBinding: string;
  deviceLimit: number;
  metadata?: Record<string, string>;
}

/** The payload exactly as it is serialised and signed. Field order is the contract. */
export interface SerializedLicensePayload {
  schemaVersion: number;
  keyId: string;
  licenseId: string;
  productId: string;
  tierId: string;
  issuedAt: number;
  permanent: boolean;
  leaseUntil: number | null;
  deviceBinding: string;
  deviceLimit: number;
  metadata: Record<string, string>;
}

/** Loads a PEM-encoded Ed25519 private key from the environment. */
export function loadPrivateKey(pem: string) {
  const normalized = pem.includes('-----BEGIN')
    ? pem
    : `-----BEGIN PRIVATE KEY-----\n${pem.replace(/\s+/g, '')}\n-----END PRIVATE KEY-----`;
  return createPrivateKey(normalized);
}

/** The public key for a private key, as PEM, for the desktop build to embed. */
export function publicKeyPem(privateKey: ReturnType<typeof createPrivateKey>): string {
  return createPublicKey(privateKey).export({ format: 'pem', type: 'spki' }).toString();
}

export function base64UrlEncode(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function base64UrlDecode(value: string): Uint8Array {
  const padded = value.replace(/-/g, '+').replace(/_/g, '/');
  const binary = atob(padded + '='.repeat((4 - (padded.length % 4)) % 4));
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

/** Serialises a payload deterministically, so the same input always yields the same bytes. */
export function serializePayload(payload: SerializedLicensePayload): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(payload));
}

/** Signs a serialized payload and returns the `<payload>.<signature>` licence key. */
export function signPayload(
  privateKey: ReturnType<typeof createPrivateKey>,
  payload: SerializedLicensePayload,
): { payloadBase64: string; signatureBase64: string; licenceKey: string } {
  const bytes = serializePayload(payload);
  // Ed25519 takes a null digest; SHA-256 here would be a different algorithm entirely.
  const signature = sign(null, Buffer.from(bytes), privateKey);
  const payloadBase64 = base64UrlEncode(bytes);
  const signatureBase64 = base64UrlEncode(new Uint8Array(signature));
  return {
    payloadBase64,
    signatureBase64,
    licenceKey: `${payloadBase64}.${signatureBase64}`,
  };
}

/** Verifies a licence key. Used by the integration tests and by the recovery flow. */
export function verifyPayload(
  publicKey: ReturnType<typeof createPublicKey>,
  payloadBase64: string,
  signatureBase64: string,
): boolean {
  try {
    const bytes = base64UrlDecode(payloadBase64);
    const signature = base64UrlDecode(signatureBase64);
    return verify(null, Buffer.from(bytes), publicKey, Buffer.from(signature));
  } catch {
    return false;
  }
}

/** Builds the payload the desktop app will verify. */
export function buildPayload(
  input: LicensePayloadInput,
  keyId: string,
  schemaVersion: number,
): SerializedLicensePayload {
  return {
    schemaVersion,
    keyId,
    licenseId: input.licenseId,
    productId: input.productId,
    tierId: input.tierId,
    issuedAt: input.issuedAt,
    permanent: input.permanent,
    leaseUntil: input.leaseUntil,
    deviceBinding: input.deviceBinding,
    deviceLimit: input.deviceLimit,
    metadata: input.metadata ?? {},
  };
}
