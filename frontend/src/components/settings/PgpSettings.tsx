"use client";

import { useState } from "react";
import { KeyRound, Trash2, Download, Plus, Loader2, Shield } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { usePgpKeys, useStorePgpKey, useDeletePgpKey } from "@/hooks/usePgp";
import { generateKey } from "@/lib/pgp/client";

type AddMode = "idle" | "generate" | "import";

export function PgpSettings() {
  const { data: keys, isLoading } = usePgpKeys();
  const storeKey = useStorePgpKey();
  const deleteKey = useDeletePgpKey();

  const [addMode, setAddMode] = useState<AddMode>("idle");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  // Generate form state
  const [genName, setGenName] = useState("");
  const [genEmail, setGenEmail] = useState("");
  const [genPassphrase, setGenPassphrase] = useState("");
  const [genPassphraseConfirm, setGenPassphraseConfirm] = useState("");
  const [generating, setGenerating] = useState(false);

  // Import form state
  const [importArmored, setImportArmored] = useState("");
  const [importPassphrase, setImportPassphrase] = useState("");
  const [importing, setImporting] = useState(false);

  function resetForms() {
    setAddMode("idle");
    setGenName("");
    setGenEmail("");
    setGenPassphrase("");
    setGenPassphraseConfirm("");
    setImportArmored("");
    setImportPassphrase("");
    setConfirmDeleteId(null);
  }

  async function handleGenerate() {
    if (genPassphrase.length < 8) {
      toast.error("Passphrase must be at least 8 characters");
      return;
    }
    if (genPassphrase !== genPassphraseConfirm) {
      toast.error("Passphrases do not match");
      return;
    }
    if (!genEmail.trim()) {
      toast.error("Email is required");
      return;
    }

    setGenerating(true);
    try {
      const { privateKeyArmored, publicKeyArmored, fingerprint } =
        await generateKey({
          email: genEmail.trim(),
          name: genName.trim(),
          passphrase: genPassphrase,
        });

      await storeKey.mutateAsync({
        id: crypto.randomUUID(),
        fingerprint,
        public_key: publicKeyArmored,
        private_key_enc: privateKeyArmored,
      });

      toast.success("PGP key generated and stored");
      resetForms();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Failed to generate key");
    } finally {
      setGenerating(false);
    }
  }

  async function handleImport() {
    if (!importArmored.trim()) {
      toast.error("Paste your armored private key");
      return;
    }
    if (!importPassphrase) {
      toast.error("Passphrase is required to verify the key");
      return;
    }

    setImporting(true);
    try {
      // Dynamically import openpgp only when needed (heavy library).
      const openpgp = await import("openpgp");
      const privKey = await openpgp.readPrivateKey({ armoredKey: importArmored.trim() });
      const decrypted = await openpgp.decryptKey({
        privateKey: privKey,
        passphrase: importPassphrase,
      });
      const fingerprint = decrypted.getFingerprint().toUpperCase();
      const publicKeyArmored = decrypted.toPublic().armor();

      await storeKey.mutateAsync({
        id: crypto.randomUUID(),
        fingerprint,
        public_key: publicKeyArmored,
        private_key_enc: importArmored.trim(),
      });

      toast.success("PGP key imported");
      resetForms();
    } catch (e) {
      toast.error(
        e instanceof Error ? e.message : "Failed to import key - check passphrase",
      );
    } finally {
      setImporting(false);
    }
  }

  async function handleDelete(id: string) {
    try {
      await deleteKey.mutateAsync(id);
      toast.success("PGP key removed");
      setConfirmDeleteId(null);
    } catch {
      toast.error("Failed to remove key");
    }
  }

  function handleExport(publicKey: string, fingerprint: string) {
    const blob = new Blob([publicKey], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `pgp-${fingerprint.slice(-8).toLowerCase()}.asc`;
    a.click();
    URL.revokeObjectURL(url);
  }

  if (isLoading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground py-8">
        <Loader2 className="size-4 animate-spin" />
        Loading PGP keys...
      </div>
    );
  }

  const keyList = keys ?? [];

  return (
    <div className="flex flex-col gap-6 max-w-2xl">
      <div>
        <h2 className="text-base font-semibold">PGP Keys</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Manage your OpenPGP keys for encrypted and signed email.
          Private keys are stored encrypted with your passphrase.
        </p>
      </div>

      {/* Key list */}
      <div className="rounded-lg border border-border p-4 flex flex-col gap-3">
        <div className="flex items-center gap-2">
          <Shield className="size-4 text-muted-foreground" />
          <span className="text-sm font-medium">Your keys</span>
          {keyList.length > 0 && (
            <span className="ml-auto text-xs text-muted-foreground">
              {keyList.length} key{keyList.length !== 1 ? "s" : ""}
            </span>
          )}
        </div>

        {keyList.length === 0 && (
          <p className="text-sm text-muted-foreground py-2">
            No PGP keys configured. Add a key to enable encrypted and signed email.
          </p>
        )}

        {keyList.map((key) => (
          <div key={key.id}>
            <div className="flex items-center justify-between rounded-md border border-border px-3 py-2">
              <div className="flex flex-col gap-0.5 min-w-0">
                <span className="text-xs font-mono text-muted-foreground truncate">
                  {key.fingerprint.match(/.{1,4}/g)?.join(" ") ?? key.fingerprint}
                </span>
                <span className="text-xs text-muted-foreground">
                  Added {new Date(key.created_at * 1000).toLocaleDateString()}
                  {key.identity_id && " - linked to identity"}
                </span>
              </div>
              <div className="flex items-center gap-1 shrink-0 ml-2">
                <Button
                  variant="ghost"
                  size="xs"
                  onClick={() => handleExport(key.public_key, key.fingerprint)}
                  title="Export public key"
                >
                  <Download className="size-3.5" />
                </Button>
                <Button
                  variant="destructive"
                  size="xs"
                  onClick={() => setConfirmDeleteId(key.id)}
                  disabled={deleteKey.isPending && confirmDeleteId === key.id}
                >
                  <Trash2 className="size-3.5" />
                  Remove
                </Button>
              </div>
            </div>

            {confirmDeleteId === key.id && (
              <div className="mt-1 rounded-md bg-destructive/10 p-3">
                <p className="text-sm text-destructive font-medium">Remove this key?</p>
                <p className="mt-1 text-xs text-destructive/80">
                  You will no longer be able to send signed or encrypted messages with it.
                </p>
                <div className="mt-3 flex gap-2">
                  <Button
                    variant="destructive"
                    size="sm"
                    onClick={() => handleDelete(key.id)}
                    disabled={deleteKey.isPending}
                  >
                    {deleteKey.isPending && <Loader2 className="size-3.5 animate-spin" />}
                    Remove
                  </Button>
                  <button
                    type="button"
                    onClick={() => setConfirmDeleteId(null)}
                    className="rounded-md px-3 py-1.5 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
                  >
                    Cancel
                  </button>
                </div>
              </div>
            )}
          </div>
        ))}
      </div>

      {/* Add key panel */}
      <div className="rounded-lg border border-border p-4">
        <div className="flex items-center gap-2 mb-3">
          <KeyRound className="size-4 text-muted-foreground" />
          <span className="text-sm font-medium">Add a key</span>
        </div>

        {addMode === "idle" && (
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => setAddMode("generate")}
              className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 transition-colors"
            >
              <Plus className="size-3.5" />
              Generate new key
            </button>
            <button
              type="button"
              onClick={() => setAddMode("import")}
              className="inline-flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-sm font-medium hover:bg-accent transition-colors"
            >
              Import existing key
            </button>
          </div>
        )}

        {addMode === "generate" && (
          <div className="space-y-3">
            <div className="grid grid-cols-2 gap-3">
              <div className="flex flex-col gap-1">
                <label className="text-xs font-medium text-muted-foreground">Name</label>
                <input
                  type="text"
                  placeholder="Your name"
                  value={genName}
                  onChange={(e) => setGenName(e.target.value)}
                  disabled={generating}
                  className="rounded-md border border-border bg-background px-3 py-1.5 text-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary disabled:opacity-50"
                />
              </div>
              <div className="flex flex-col gap-1">
                <label className="text-xs font-medium text-muted-foreground">Email</label>
                <input
                  type="email"
                  placeholder="you@example.com"
                  value={genEmail}
                  onChange={(e) => setGenEmail(e.target.value)}
                  disabled={generating}
                  className="rounded-md border border-border bg-background px-3 py-1.5 text-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary disabled:opacity-50"
                />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div className="flex flex-col gap-1">
                <label className="text-xs font-medium text-muted-foreground">
                  Passphrase <span className="text-destructive">*</span>
                </label>
                <input
                  type="password"
                  placeholder="Min. 8 characters"
                  value={genPassphrase}
                  onChange={(e) => setGenPassphrase(e.target.value)}
                  disabled={generating}
                  className="rounded-md border border-border bg-background px-3 py-1.5 text-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary disabled:opacity-50"
                />
              </div>
              <div className="flex flex-col gap-1">
                <label className="text-xs font-medium text-muted-foreground">
                  Confirm passphrase
                </label>
                <input
                  type="password"
                  placeholder="Repeat passphrase"
                  value={genPassphraseConfirm}
                  onChange={(e) => setGenPassphraseConfirm(e.target.value)}
                  disabled={generating}
                  className={cn(
                    "rounded-md border bg-background px-3 py-1.5 text-sm focus:outline-none focus:ring-1 disabled:opacity-50",
                    genPassphraseConfirm && genPassphrase !== genPassphraseConfirm
                      ? "border-destructive focus:border-destructive focus:ring-destructive"
                      : "border-border focus:border-primary focus:ring-primary",
                  )}
                />
              </div>
            </div>
            <div className="flex gap-2">
              <button
                type="button"
                onClick={handleGenerate}
                disabled={generating}
                className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50 transition-colors"
              >
                {generating ? (
                  <>
                    <Loader2 className="size-3.5 animate-spin" />
                    Generating...
                  </>
                ) : (
                  <>
                    <KeyRound className="size-3.5" />
                    Generate key
                  </>
                )}
              </button>
              <button
                type="button"
                onClick={resetForms}
                disabled={generating}
                className="rounded-md px-3 py-1.5 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {addMode === "import" && (
          <div className="space-y-3">
            <div className="flex flex-col gap-1">
              <label className="text-xs font-medium text-muted-foreground">
                Armored private key
              </label>
              <textarea
                rows={6}
                placeholder="-----BEGIN PGP PRIVATE KEY BLOCK-----"
                value={importArmored}
                onChange={(e) => setImportArmored(e.target.value)}
                disabled={importing}
                className="rounded-md border border-border bg-background px-3 py-2 text-sm font-mono focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary disabled:opacity-50 resize-none"
              />
            </div>
            <div className="flex flex-col gap-1">
              <label className="text-xs font-medium text-muted-foreground">
                Passphrase
              </label>
              <input
                type="password"
                placeholder="Key passphrase"
                value={importPassphrase}
                onChange={(e) => setImportPassphrase(e.target.value)}
                disabled={importing}
                className="rounded-md border border-border bg-background px-3 py-1.5 text-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary disabled:opacity-50"
              />
            </div>
            <div className="flex gap-2">
              <button
                type="button"
                onClick={handleImport}
                disabled={importing}
                className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50 transition-colors"
              >
                {importing ? (
                  <>
                    <Loader2 className="size-3.5 animate-spin" />
                    Importing...
                  </>
                ) : (
                  "Import key"
                )}
              </button>
              <button
                type="button"
                onClick={resetForms}
                disabled={importing}
                className="rounded-md px-3 py-1.5 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
              >
                Cancel
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
