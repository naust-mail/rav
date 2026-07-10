/** Optional capability flags exposed by the server's health endpoint. */
export type ServerCapability = 'pgp';

/** Response shape of `GET /api/health`. */
export type HealthResponse = {
  status: string;
  capabilities: ServerCapability[];
};

/** A stored PGP key summary returned by the list endpoint (no private key material). */
export type PgpKeySummary = {
  id: string;
  identity_id: number | null;
  fingerprint: string;
  public_key: string;
  created_at: number;
};

/** A full PGP key record including the passphrase-protected private key blob. */
export type PgpKeyRecord = PgpKeySummary & {
  private_key_enc: string;
};

// Generated from the Rust backend (backend/src/imap/types.rs) via ts-rs.
// Regenerate with `cargo test --features ts-export` in backend/.
export type { PgpMessageStatus } from "./generated/PgpMessageStatus";
export type { PgpStatusKind } from "./generated/PgpStatusKind";

/** PGP parameters included in an outbound send request. */
export type PgpSendRequest = {
  mode: 'sign' | 'encrypt';
  signature: string | null;
  ciphertext: string | null;
  micalg: string;
};

/** WKD key discovery response from the backend proxy. */
export type WkdLookupResponse = {
  found: boolean;
  /** Armored public key text or base64-encoded binary key data. */
  public_key: string | null;
};

/** Result of a client-side decrypt operation. */
export type DecryptResult = {
  text: string;
  html: string | null;
  verified: 'valid' | 'invalid' | 'unsigned' | 'no_key';
};

/** Result of a client-side sign operation. */
export type SignResult = {
  signature: string;
  micalg: string;
};
