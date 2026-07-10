import type { QueryClient } from "@tanstack/react-query";
import type { FolderId } from "@/types/folder";
import type { FoldersResponse } from "@/types/folder";

/**
 * Resolve a folder name to its current FolderId by reading the warm
 * `["folders"]` query cache synchronously - no network request, safe to call
 * from inside a queryFn/mutationFn (unlike a hook, which cannot be called
 * outside render).
 *
 * FolderIds are single-use and non-deterministic (a fresh one is minted on
 * every server response), so this is the only correct way to go from a
 * folder *name* to a usable id. If you already have a record (Folder,
 * MessageHeader, MessageDetail, SearchResultItem) with its own `folder_id`,
 * use that directly instead - it's already valid and this lookup is
 * unnecessary.
 *
 * Throws if the folders list hasn't loaded yet or the name doesn't match any
 * known folder, rather than silently sending a request with "undefined" in
 * the URL.
 */
export function resolveFolderId(queryClient: QueryClient, name: string): FolderId {
  const folders = queryClient.getQueryData<FoldersResponse>(["folders"])?.folders;
  const id = folders?.find((f) => f.name === name)?.id;
  if (!id) {
    throw new Error(`Unknown folder "${name}" - folder list not loaded or folder does not exist`);
  }
  return id;
}
