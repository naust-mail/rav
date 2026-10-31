/** Response from POST /api/auth/password. */
export type ChangePasswordResponse = {
  status: "changed";
  /** False when the mail server did not accept the new password yet (e.g. a short auth cache). */
  verified: boolean;
  /** Passkeys removed because they were tied to the old password. */
  passkeysRemoved: number;
  /** Sessions of this mailbox in other browsers that were signed out. */
  otherSessionsSignedOut: number;
};
