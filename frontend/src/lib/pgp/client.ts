"use client";

import type { DecryptResult, SignResult } from '@/types/pgp';

type PendingRequest = {
  resolve: (value: unknown) => void;
  reject: (reason: Error) => void;
};

type WorkerMessage =
  | { id: string; ok: true; result: unknown }
  | { id: string; ok: false; error: string };

// Singleton worker instance, initialized on first use.
let worker: Worker | null = null;
const pending = new Map<string, PendingRequest>();

function getWorker(): Worker {
  if (!worker) {
    worker = new Worker(new URL('./worker.ts', import.meta.url));
    worker.onmessage = (event: MessageEvent<WorkerMessage>) => {
      const msg = event.data;
      const handlers = pending.get(msg.id);
      if (!handlers) return;
      pending.delete(msg.id);
      if (msg.ok) {
        handlers.resolve(msg.result);
      } else {
        handlers.reject(new Error(msg.error));
      }
    };
    worker.onerror = (err) => {
      // Reject all pending requests if the worker crashes.
      const error = new Error(`PGP worker error: ${err.message}`);
      for (const handlers of pending.values()) {
        handlers.reject(error);
      }
      pending.clear();
      worker = null;
    };
  }
  return worker;
}

function call<T>(request: Record<string, unknown>): Promise<T> {
  const id = crypto.randomUUID();
  return new Promise<T>((resolve, reject) => {
    pending.set(id, {
      resolve: resolve as (value: unknown) => void,
      reject,
    });
    try {
      getWorker().postMessage({ id, ...request });
    } catch (e) {
      pending.delete(id);
      reject(e instanceof Error ? e : new Error(String(e)));
    }
  });
}

export function generateKey(params: {
  email: string;
  name: string;
  passphrase: string;
}): Promise<{ privateKeyArmored: string; publicKeyArmored: string; fingerprint: string }> {
  return call({ type: 'generate', ...params });
}

export function decryptMessage(params: {
  privateKeyArmored: string;
  passphrase: string;
  ciphertext: string;
  /** Sender's public key (armored) for verifying an embedded signature. */
  senderPublicKeyArmored?: string;
}): Promise<DecryptResult> {
  return call({ type: 'decrypt', ...params });
}

export function signContent(params: {
  privateKeyArmored: string;
  passphrase: string;
  content: string;
}): Promise<SignResult> {
  return call({ type: 'sign', ...params });
}

export function verifySignature(params: {
  publicKeyArmored: string;
  /** Base64-encoded raw MIME bytes of the signed body part (RFC 3156). */
  contentB64: string;
  signature: string;
}): Promise<{ verified: 'valid' | 'invalid' }> {
  return call({ type: 'verify', ...params });
}

export function encryptMessage(params: {
  publicKeyArmoreds: string[];
  content: string;
  privateKeyArmored?: string;
  passphrase?: string;
}): Promise<{ ciphertext: string }> {
  return call({ type: 'encrypt', ...params });
}
