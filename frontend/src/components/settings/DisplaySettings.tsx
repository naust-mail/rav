"use client";

import { useCallback } from "react";
import { Code2, FileText, Loader2, Monitor, Moon, Search, Sun } from "lucide-react";
import { toast } from "sonner";
import {
  useDisplayPreferences,
  useUpdateDisplayPreferences,
  parseMobileNavTabs,
} from "@/hooks/useDisplayPreferences";
import { runThemeSpreadTransition } from "@/lib/motion/theme-spread";
import { useUiStore } from "@/stores/useUiStore";
import type { ThemeMode } from "@/stores/useUiStore";
import type { AnimationMode } from "@/lib/motion/config";
import { cn } from "@/lib/utils";

function SegmentedControl<T extends string>({
  label,
  description,
  value,
  options,
  onChange,
}: {
  label: string;
  description: string;
  value: T;
  options: { value: T; label: string; icon?: React.ReactNode }[];
  onChange: (value: T) => void;
}) {
  return (
    <div className="flex items-center justify-between rounded-lg border border-border p-4">
      <div>
        <div className="text-sm font-medium">{label}</div>
        <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>
      </div>
      <div className="flex rounded-lg border border-border bg-muted/50 p-0.5">
        {options.map((opt) => (
          <button
            key={opt.value}
            type="button"
            onClick={() => onChange(opt.value)}
            className={cn(
              "flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors",
              value === opt.value
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {opt.icon}
            {opt.label}
          </button>
        ))}
      </div>
    </div>
  );
}

export function DisplaySettings() {
  const { data: prefs, isLoading } = useDisplayPreferences();
  const updatePrefs = useUpdateDisplayPreferences();
  const setDensity = useUiStore((s) => s.setDensity);
  const setTheme = useUiStore((s) => s.setTheme);
  const setComposeFormat = useUiStore((s) => s.setComposeFormat);
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);

  const handleDensityChange = useCallback(
    (density: "compact" | "comfortable") => {
      setDensity(density);
      updatePrefs.mutate(
        { density },
        { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
      );
    },
    [setDensity, updatePrefs],
  );

  const handleThemeChange = useCallback(
    (theme: ThemeMode) => {
      runThemeSpreadTransition({
        mode: effectiveAnimationMode,
        trigger: "explicit",
        applyTheme: () => setTheme(theme),
        nextTheme: theme,
      });
      updatePrefs.mutate(
        { theme },
        { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
      );
    },
    [effectiveAnimationMode, setTheme, updatePrefs],
  );

  const handleComposeFormatChange = useCallback(
    (compose_format: "html" | "text") => {
      setComposeFormat(compose_format);
      updatePrefs.mutate(
        { compose_format },
        { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
      );
    },
    [setComposeFormat, updatePrefs],
  );

  const handleAnimationModeChange = useCallback(
    (animation_mode: AnimationMode) => {
      updatePrefs.mutate(
        { animation_mode },
        { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
      );
    },
    [updatePrefs],
  );

  const handleReset = useCallback(() => {
    setDensity("comfortable");
    setTheme("system");
    setComposeFormat("html");
    updatePrefs.mutate(
      {
        density: "comfortable",
        theme: "system",
        language: "en",
        compose_format: "html",
        animation_mode: null,
        deep_index: false,
        mobile_nav_tabs: null,
        mobile_compose: null,
      },
      { onError: (e) => toast.error(`Failed to reset: ${e.message}`) },
    );
  }, [setDensity, setTheme, setComposeFormat, updatePrefs]);

  if (isLoading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="size-4 animate-spin" />
        Loading display settings...
      </div>
    );
  }

  if (!prefs) return null;

  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h2 className="text-base font-semibold">Display</h2>
        <p className="mt-0.5 text-sm text-muted-foreground">
          Customize the appearance and layout of the app.
        </p>
      </div>

      <div className="space-y-3">
        <SegmentedControl
          label="Density"
          description="Controls the spacing of the message list"
          value={prefs.density}
          options={[
            { value: "compact", label: "Compact" },
            { value: "comfortable", label: "Comfortable" },
          ]}
          onChange={handleDensityChange}
        />

        <SegmentedControl
          label="Theme"
          description="Choose light, dark, or match your system"
          value={prefs.theme}
          options={[
            { value: "light", label: "Light", icon: <Sun className="size-3.5" /> },
            { value: "dark", label: "Dark", icon: <Moon className="size-3.5" /> },
            { value: "system", label: "System", icon: <Monitor className="size-3.5" /> },
          ]}
          onChange={handleThemeChange}
        />

        <SegmentedControl
          label="Compose format"
          description="Default format for new emails"
          value={prefs.compose_format}
          options={[
            { value: "html", label: "HTML", icon: <Code2 className="size-3.5" /> },
            { value: "text", label: "Plain text", icon: <FileText className="size-3.5" /> },
          ]}
          onChange={handleComposeFormatChange}
        />

        <SegmentedControl
          label="Animations"
          description="Choose how much motion to use in the interface"
          value={prefs.animation_mode ?? "medium"}
          options={[
            { value: "rich", label: "Rich" },
            { value: "medium", label: "Medium" },
            { value: "subtle", label: "Subtle" },
            { value: "off", label: "Off" },
          ]}
          onChange={handleAnimationModeChange}
        />

        <SegmentedControl
          label="Search indexing"
          description="Index message bodies for full-text search (uses more bandwidth)"
          value={prefs.deep_index ? "on" : "off"}
          options={[
            { value: "off", label: "Headers only", icon: <Search className="size-3.5" /> },
            { value: "on", label: "Full text", icon: <Search className="size-3.5" /> },
          ]}
          onChange={(v) =>
            updatePrefs.mutate(
              { deep_index: v === "on" },
              { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
            )
          }
        />

        <div className="flex items-center justify-between rounded-lg border border-border p-4">
          <div>
            <div className="text-sm font-medium">Language</div>
            <p className="mt-0.5 text-xs text-muted-foreground">
              Interface language
            </p>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-sm text-muted-foreground">English</span>
            <span className="rounded-full bg-muted px-2 py-0.5 text-[10px] text-muted-foreground">
              More coming soon
            </span>
          </div>
        </div>
      </div>

      <MobileNavSection prefs={prefs} updatePrefs={updatePrefs} />

      <button
        type="button"
        onClick={handleReset}
        className="text-sm text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
      >
        Reset to defaults
      </button>
    </div>
  );
}

const OPTIONAL_TABS = ["calendar", "contacts", "search", "compose"] as const;
type OptionalTab = (typeof OPTIONAL_TABS)[number];

function MobileNavSection({
  prefs,
  updatePrefs,
}: {
  prefs: ReturnType<typeof useDisplayPreferences>["data"];
  updatePrefs: ReturnType<typeof useUpdateDisplayPreferences>;
}) {
  if (!prefs) return null;

  const enabledTabs = parseMobileNavTabs(prefs.mobile_nav_tabs);
  const composeMode = prefs.mobile_compose ?? "fab";

  const handleTabToggle = (tab: OptionalTab) => {
    const isCompose = tab === "compose";
    if (isCompose) {
      // Toggle compose in tabs vs FAB
      const newMode = composeMode === "tab" ? "fab" : "tab";
      updatePrefs.mutate(
        { mobile_compose: newMode },
        { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
      );
      return;
    }

    const next = enabledTabs.includes(tab)
      ? enabledTabs.filter((t) => t !== tab)
      : [...enabledTabs, tab];

    updatePrefs.mutate(
      { mobile_nav_tabs: JSON.stringify(next) },
      { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
    );
  };

  // Max 3 optional tabs (compose counts if it's in tabs)
  const optionalCount = enabledTabs.filter((t) => t !== "compose").length + (composeMode === "tab" ? 1 : 0);
  const atMax = optionalCount >= 3;

  return (
    <div className="space-y-3">
      <div>
        <h3 className="text-sm font-semibold">Mobile navigation</h3>
        <p className="mt-0.5 text-xs text-muted-foreground">
          Configure the bottom navigation on small screens.
        </p>
      </div>

      <div className="rounded-lg border border-border p-4 space-y-3">
          <div>
            <div className="text-sm font-medium">Tab items</div>
            <p className="mt-0.5 text-xs text-muted-foreground">
              Mail is always first. Choose up to 3 additional tabs.
            </p>
          </div>
          <div className="space-y-2">
            {OPTIONAL_TABS.map((tab) => {
              const isCompose = tab === "compose";
              const checked = isCompose ? composeMode === "tab" : enabledTabs.includes(tab);
              const disabled = !checked && atMax;

              return (
                <label
                  key={tab}
                  className={cn(
                    "flex items-start gap-3 cursor-pointer",
                    disabled && "opacity-40 cursor-not-allowed",
                  )}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    disabled={disabled}
                    onChange={() => handleTabToggle(tab)}
                    className="mt-0.5 size-4 rounded border-border accent-primary"
                  />
                  <div>
                    <span className="text-sm font-medium capitalize">{tab}</span>
                    {isCompose && (
                      <p className="text-xs text-muted-foreground">
                        Adds Compose to the tab bar and removes the floating button.
                      </p>
                    )}
                  </div>
                </label>
              );
            })}
          </div>
        </div>

      {composeMode !== "tab" && (
        <SegmentedControl
          label="Compose button"
          description="Where to place the compose action when it is not in the tab bar"
          value={composeMode}
          options={[{ value: "fab", label: "Floating button (FAB)" }]}
          onChange={(v) =>
            updatePrefs.mutate(
              { mobile_compose: v },
              { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
            )
          }
        />
      )}
    </div>
  );
}
