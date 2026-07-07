import { apiGet } from '@/lib/api';
import type { WkdLookupResponse } from '@/types/pgp';

/** Look up a public key via WKD (Web Key Directory) for the given email address.
 *  Returns the key as an armored string or base64-encoded binary, or null if not found. */
export async function lookupWkd(email: string): Promise<string | null> {
  const res = await apiGet<WkdLookupResponse>(
    `/pgp/wkd?email=${encodeURIComponent(email)}`,
  );
  return res.found ? res.public_key : null;
}
