import { describe, it, expect, vi, beforeEach } from 'vitest';
import { lookupWkd } from '../wkd';
import * as api from '@/lib/api';

vi.mock('@/lib/api');

const mockedApiGet = vi.mocked(api.apiGet);

beforeEach(() => {
  vi.clearAllMocks();
});

describe('lookupWkd', () => {
  it('returns public key when found', async () => {
    mockedApiGet.mockResolvedValueOnce({ found: true, public_key: '-----BEGIN PGP PUBLIC KEY BLOCK-----\n...' });
    const result = await lookupWkd('alice@example.com');
    expect(result).toBe('-----BEGIN PGP PUBLIC KEY BLOCK-----\n...');
  });

  it('returns null when not found', async () => {
    mockedApiGet.mockResolvedValueOnce({ found: false, public_key: null });
    const result = await lookupWkd('nobody@example.com');
    expect(result).toBeNull();
  });

  it('encodes the email in the query string', async () => {
    mockedApiGet.mockResolvedValueOnce({ found: false, public_key: null });
    await lookupWkd('user+tag@example.com');
    expect(mockedApiGet).toHaveBeenCalledWith(
      '/pgp/wkd?email=user%2Btag%40example.com',
    );
  });
});
