import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// Mock the Worker global before importing client
const mockPostMessage = vi.fn();
const mockWorkerInstance = {
  postMessage: mockPostMessage,
  onmessage: null as ((e: MessageEvent) => void) | null,
  onerror: null as ((e: ErrorEvent) => void) | null,
};

vi.stubGlobal('Worker', vi.fn(() => mockWorkerInstance));

// Import after mocking
const { generateKey, decryptMessage, signContent, verifySignature, encryptMessage } =
  await import('../client');

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  // Reset worker singleton between tests by clearing module cache.
  // Since we stub Worker globally, each test gets a fresh call.
});

function resolveWorkerCall(result: unknown) {
  const call = mockPostMessage.mock.calls[0];
  const id: string = call[0].id;
  const handler = mockWorkerInstance.onmessage;
  if (handler) {
    handler(new MessageEvent('message', { data: { id, ok: true, result } }));
  }
}

function rejectWorkerCall(errorMsg: string) {
  const call = mockPostMessage.mock.calls[0];
  const id: string = call[0].id;
  const handler = mockWorkerInstance.onmessage;
  if (handler) {
    handler(new MessageEvent('message', { data: { id, ok: false, error: errorMsg } }));
  }
}

describe('generateKey', () => {
  it('sends correct message type to worker', async () => {
    const params = { email: 'a@b.com', name: 'Alice', passphrase: 'secret123' };
    const expectedResult = {
      privateKeyArmored: '-----BEGIN PGP PRIVATE KEY BLOCK-----',
      publicKeyArmored: '-----BEGIN PGP PUBLIC KEY BLOCK-----',
      fingerprint: 'ABCD1234',
    };

    const promise = generateKey(params);
    resolveWorkerCall(expectedResult);

    const result = await promise;
    expect(mockPostMessage).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'generate', email: 'a@b.com', name: 'Alice' }),
    );
    expect(result).toEqual(expectedResult);
  });
});

describe('decryptMessage', () => {
  it('sends correct message type to worker', async () => {
    const params = {
      privateKeyArmored: '-----BEGIN PGP PRIVATE KEY BLOCK-----',
      passphrase: 'pass',
      ciphertext: '-----BEGIN PGP MESSAGE-----',
    };

    const promise = decryptMessage(params);
    resolveWorkerCall({ text: 'Hello world', html: null, verified: 'unsigned' });

    const result = await promise;
    expect(mockPostMessage).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'decrypt' }),
    );
    expect(result.text).toBe('Hello world');
  });
});

describe('worker error handling', () => {
  it('rejects with an Error when worker returns ok: false', async () => {
    const promise = signContent({
      privateKeyArmored: 'key',
      passphrase: 'pass',
      content: 'hello',
    });
    rejectWorkerCall('Bad passphrase');
    await expect(promise).rejects.toThrow('Bad passphrase');
  });
});

describe('verifySignature', () => {
  it('sends correct message type and returns verified status', async () => {
    const params = {
      publicKeyArmored: '-----BEGIN PGP PUBLIC KEY BLOCK-----',
      contentB64: btoa('hello'),
      signature: '-----BEGIN PGP SIGNATURE-----',
    };

    const promise = verifySignature(params);
    resolveWorkerCall({ verified: 'valid' });

    const result = await promise;
    expect(mockPostMessage).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'verify', contentB64: btoa('hello') }),
    );
    expect(result.verified).toBe('valid');
  });

  it('returns invalid when signature does not match', async () => {
    const promise = verifySignature({
      publicKeyArmored: '-----BEGIN PGP PUBLIC KEY BLOCK-----',
      contentB64: btoa('tampered'),
      signature: '-----BEGIN PGP SIGNATURE-----',
    });
    resolveWorkerCall({ verified: 'invalid' });

    const result = await promise;
    expect(result.verified).toBe('invalid');
  });
});

describe('encryptMessage', () => {
  it('sends public keys and content to worker', async () => {
    const promise = encryptMessage({
      publicKeyArmoreds: ['-----BEGIN PGP PUBLIC KEY BLOCK-----'],
      content: 'secret message',
    });
    resolveWorkerCall({ ciphertext: '-----BEGIN PGP MESSAGE-----' });

    const result = await promise;
    expect(mockPostMessage).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'encrypt' }),
    );
    expect(result.ciphertext).toBe('-----BEGIN PGP MESSAGE-----');
  });
});
