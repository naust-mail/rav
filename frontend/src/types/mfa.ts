/** MFA status for the authenticated user. */
export type MfaStatus = {
  totp_enabled: boolean;
  /** Number of enrolled passkeys. */
  passkey_count: number;
  /** When true, password and TOTP login are disabled for this account. */
  passkey_only: boolean;
};

/** Response from POST /api/mfa/totp/setup. */
export type TotpSetupResponse = {
  /** Base32-encoded TOTP secret. Must be sent back at confirm time. */
  secret: string;
  /** otpauth:// URI for QR code generation or direct app linking. */
  url: string;
};

/** Response from POST /api/mfa/totp/confirm and DELETE /api/mfa/totp. */
export type TotpToggleResponse = {
  totp_enabled: boolean;
};

/** Login response when TOTP is required. The server has not verified the
 *  password at this point; no auth factor has been confirmed. */
export type LoginMfaRequired = {
  mfa_required: true;
  mfa_type: "totp";
};

/** Login response on full success (all factors verified). */
export type LoginSuccess = {
  account: {
    id: string;
    email: string;
    imapHost: string;
    smtpHost: string;
  };
};

export type LoginResponse = LoginMfaRequired | LoginSuccess;

/** Credential descriptor in passkey request options (id is base64url). */
type PasskeyAllowCredential = {
  type: string;
  id: string;
};

/** publicKey options from the server for navigator.credentials.get() (binary fields are base64url). */
type PasskeyPublicKeyRequestOptions = {
  challenge: string;
  timeout?: number;
  rpId?: string;
  allowCredentials?: PasskeyAllowCredential[];
  userVerification?: string;
  extensions?: Record<string, unknown>;
};

/** Response from POST /api/auth/mfa/passkey/login/begin. */
export type PasskeyLoginBeginResponse = {
  nonce: string;
  options: {
    publicKey: PasskeyPublicKeyRequestOptions;
  };
};

/** Serialized authenticator assertion sent to POST /api/auth/mfa/passkey/login/complete.
 *  All binary fields are base64url-encoded strings. */
export type PasskeyAssertionResponse = {
  id: string;
  rawId: string;
  type: string;
  response: {
    authenticatorData: string;
    clientDataJSON: string;
    signature: string;
    userHandle: string | null;
  };
  /** PRF output from the authenticator, used server-side to decrypt the IMAP password. */
  clientExtensionResults: {
    prf?: {
      results?: {
        first?: string;
      };
    };
  };
};

/** A single enrolled passkey returned by GET /mfa/passkeys. */
export type PasskeyInfo = {
  /** Base64url-encoded credential ID. */
  id: string;
  name: string;
  /** ISO 8601 creation timestamp. */
  created_at: string;
};

/** Response from GET /mfa/passkeys. */
export type PasskeyListResponse = {
  passkeys: PasskeyInfo[];
};

/** publicKey creation options from server for navigator.credentials.create() (binary fields are base64url). */
type PasskeyPublicKeyCreationOptions = {
  challenge: string;
  rp: { name: string; id?: string };
  user: { id: string; name: string; displayName: string };
  pubKeyCredParams: Array<{ type: string; alg: number }>;
  timeout?: number;
  excludeCredentials?: Array<{ type: string; id: string }>;
  authenticatorSelection?: Record<string, unknown>;
  attestation?: string;
  extensions?: Record<string, unknown>;
};

/** Response from POST /mfa/passkey/register/begin. */
export type PasskeyRegisterBeginResponse = {
  nonce: string;
  options: { publicKey: PasskeyPublicKeyCreationOptions };
};

/** Serialized creation credential sent to POST /mfa/passkey/register/complete.
 *  All binary fields are base64url-encoded. */
export type PasskeyCreationCredential = {
  id: string;
  rawId: string;
  type: string;
  response: {
    clientDataJSON: string;
    attestationObject: string;
    transports?: string[];
  };
  /** PRF output used server-side to encrypt the IMAP password. */
  clientExtensionResults: {
    prf?: { results?: { first?: string } };
  };
};

/** Response from POST /mfa/passkey/register/complete. */
export type PasskeyRegisterCompleteResponse = {
  id: string;
  name: string;
};

/** Response from PUT /mfa/settings/passkey-only. */
export type PasskeyOnlyResponse = {
  passkey_only: boolean;
};
