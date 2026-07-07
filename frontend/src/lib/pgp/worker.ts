import * as openpgp from 'openpgp';
import type { DecryptResult, SignResult } from '@/types/pgp';

type WorkerRequest =
  | { type: 'generate'; name: string; email: string; passphrase: string }
  | {
      type: 'decrypt';
      privateKeyArmored: string;
      passphrase: string;
      ciphertext: string;
      /** Optional sender public key (armored) for verifying an embedded signature. */
      senderPublicKeyArmored?: string;
    }
  | { type: 'sign'; privateKeyArmored: string; passphrase: string; content: string }
  | { type: 'verify'; publicKeyArmored: string; contentB64: string; signature: string }
  | {
      type: 'encrypt';
      publicKeyArmoreds: string[];
      content: string;
      privateKeyArmored?: string;
      passphrase?: string;
    };

type WorkerResponse =
  | { id: string; ok: true; result: unknown }
  | { id: string; ok: false; error: string };

self.onmessage = async (event: MessageEvent<{ id: string } & WorkerRequest>) => {
  const { id, ...req } = event.data;
  try {
    let result: unknown;

    if (req.type === 'generate') {
      const { privateKey, publicKey } = await openpgp.generateKey({
        type: 'curve25519',
        userIDs: [{ name: req.name, email: req.email }],
        passphrase: req.passphrase,
        format: 'armored',
      });
      const pubKeyObj = await openpgp.readKey({ armoredKey: publicKey });
      result = {
        privateKeyArmored: privateKey,
        publicKeyArmored: publicKey,
        fingerprint: pubKeyObj.getFingerprint().toUpperCase(),
      };
    } else if (req.type === 'decrypt') {
      const privKey = await openpgp.decryptKey({
        privateKey: await openpgp.readPrivateKey({ armoredKey: req.privateKeyArmored }),
        passphrase: req.passphrase,
      });
      // Use the sender's public key for embedded signature verification when available.
      // Falling back to the recipient's own key would only work for self-sent mail.
      const verificationKey = req.senderPublicKeyArmored
        ? await openpgp.readKey({ armoredKey: req.senderPublicKeyArmored })
        : undefined;
      const message = await openpgp.readMessage({ armoredMessage: req.ciphertext });
      const { data: text, signatures } = await openpgp.decrypt({
        message,
        decryptionKeys: privKey,
        verificationKeys: verificationKey ? [verificationKey] : undefined,
      });
      let verified: DecryptResult['verified'] = 'unsigned';
      if (signatures.length > 0) {
        if (!verificationKey) {
          verified = 'no_key';
        } else {
          const ok = await signatures[0].verified.catch(() => false);
          verified = ok ? 'valid' : 'invalid';
        }
      }
      const decryptResult: DecryptResult = {
        text: stripMimeHeaders(String(text)),
        html: null,
        verified,
      };
      result = decryptResult;
    } else if (req.type === 'sign') {
      const privKey = await openpgp.decryptKey({
        privateKey: await openpgp.readPrivateKey({ armoredKey: req.privateKeyArmored }),
        passphrase: req.passphrase,
      });
      const canonical = toCanonical(req.content);
      const message = await openpgp.createMessage({ text: canonical });
      const signature = await openpgp.sign({
        message,
        signingKeys: privKey,
        detached: true,
        format: 'armored',
      });
      const signResult: SignResult = {
        signature: String(signature),
        micalg: 'pgp-sha256',
      };
      result = signResult;
    } else if (req.type === 'verify') {
      const pubKey = await openpgp.readKey({ armoredKey: req.publicKeyArmored });
      // RFC 3156: signatures are over raw MIME bytes, not text. Decode base64 to binary.
      const rawBytes = Uint8Array.from(atob(req.contentB64), (c) => c.charCodeAt(0));
      const message = await openpgp.createMessage({ binary: rawBytes });
      const sig = await openpgp.readSignature({ armoredSignature: req.signature });
      const verifyResult = await openpgp.verify({
        message,
        signature: sig,
        verificationKeys: pubKey,
      });
      const valid = await verifyResult.signatures[0]?.verified.catch(() => false);
      result = { verified: valid ? 'valid' : 'invalid' };
    } else if (req.type === 'encrypt') {
      const pubKeys = await Promise.all(
        req.publicKeyArmoreds.map((a) => openpgp.readKey({ armoredKey: a })),
      );
      let signingKey: openpgp.PrivateKey | undefined;
      if (req.privateKeyArmored && req.passphrase) {
        signingKey = await openpgp.decryptKey({
          privateKey: await openpgp.readPrivateKey({ armoredKey: req.privateKeyArmored }),
          passphrase: req.passphrase,
        });
      }
      const innerMime = buildInnerMime(req.content);
      const message = await openpgp.createMessage({ text: innerMime });
      const ciphertext = await openpgp.encrypt({
        message,
        encryptionKeys: pubKeys,
        signingKeys: signingKey ? [signingKey] : undefined,
        format: 'armored',
      });
      result = { ciphertext: String(ciphertext) };
    }

    self.postMessage({ id, ok: true, result } satisfies WorkerResponse);
  } catch (e) {
    self.postMessage({ id, ok: false, error: String(e) } satisfies WorkerResponse);
  }
};

function toCanonical(text: string): string {
  return text.replace(/\r\n/g, '\n').replace(/\r/g, '\n').replace(/\n/g, '\r\n');
}

/**
 * Strip MIME headers from decrypted PGP/MIME content.
 * The decrypted payload of a PGP/MIME encrypted message begins with
 * MIME headers (Content-Type, Content-Transfer-Encoding, etc.) followed
 * by a blank line, then the actual body. This extracts only the body.
 */
function stripMimeHeaders(raw: string): string {
  const normalized = raw.replace(/\r\n/g, '\n');
  const blankLine = normalized.indexOf('\n\n');
  if (blankLine === -1) return raw;
  return normalized.slice(blankLine + 2);
}

function buildInnerMime(textBody: string): string {
  const canonical = toCanonical(textBody);
  return `Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n${canonical}`;
}
