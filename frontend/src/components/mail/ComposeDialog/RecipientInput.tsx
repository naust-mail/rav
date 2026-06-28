"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useAutocomplete } from "@/hooks/useAutocomplete";
import { Chip } from "@/components/ui/Chip";
import { cn } from "@/lib/utils";

type RecipientInputProps = {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  inputRef?: React.Ref<HTMLInputElement>;
};

/** Highlighted suggestion text - renders fuzzy match <mark> tags from search results. */
function HighlightedText({ html }: { html: string }) {
  return (
    <span
      dangerouslySetInnerHTML={{ __html: html }}
      className="[&_mark]:bg-primary/20 [&_mark]:font-semibold [&_mark]:text-primary"
    />
  );
}

function parseRecipients(value: string): string[] {
  return value.split(",").map((s) => s.trim()).filter(Boolean);
}

function serializeRecipients(tokens: string[]): string {
  return tokens.join(", ");
}

export function RecipientInput({ value, onChange, placeholder, inputRef }: RecipientInputProps) {
  const [inputValue, setInputValue] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  const tokens = useMemo(() => parseRecipients(value), [value]);

  const query = inputValue.trim();
  const { results } = useAutocomplete(query);
  const hasResults = results.length > 0 && query.length >= 2;
  const showDropdown = dropdownOpen && hasResults;

  const commitToken = useCallback(
    (token: string) => {
      const trimmed = token.trim();
      if (!trimmed) return;
      onChange(serializeRecipients([...tokens, trimmed]));
      setInputValue("");
      setDropdownOpen(false);
      setSelectedIndex(0);
    },
    [tokens, onChange],
  );

  const removeToken = useCallback(
    (index: number) => {
      onChange(serializeRecipients(tokens.filter((_, i) => i !== index)));
    },
    [tokens, onChange],
  );

  const selectSuggestion = useCallback(
    (item: { email: string; name?: string }) => {
      const formatted =
        item.name ? `${item.name} <${item.email}>` : item.email;
      commitToken(formatted);
    },
    [commitToken],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "ArrowDown" && showDropdown) {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, results.length - 1));
        return;
      }
      if (e.key === "ArrowUp" && showDropdown) {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
        return;
      }
      if ((e.key === "Enter" || e.key === "Tab") && showDropdown) {
        e.preventDefault();
        selectSuggestion(results[selectedIndex].item);
        return;
      }
      if (e.key === "," || e.key === "Enter") {
        if (inputValue.trim()) {
          e.preventDefault();
          commitToken(inputValue);
        }
        return;
      }
      if (e.key === "Backspace" && inputValue === "" && tokens.length > 0) {
        removeToken(tokens.length - 1);
        return;
      }
      if (e.key === "Escape") {
        setDropdownOpen(false);
      }
    },
    [inputValue, tokens, showDropdown, results, selectedIndex, commitToken, removeToken, selectSuggestion],
  );

  // Commit pending input and close dropdown on outside click
  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        if (inputValue.trim()) commitToken(inputValue);
        setDropdownOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [inputValue, commitToken]);

  return (
    <div ref={containerRef} className="relative flex flex-1 flex-wrap items-center gap-1 py-1.5">
      {tokens.map((token, i) => (
        <Chip key={`${i}-${token}`} onRemove={() => removeToken(i)}>
          {token}
        </Chip>
      ))}
      <input
        ref={inputRef}
        type="text"
        value={inputValue}
        onChange={(e) => {
          setInputValue(e.target.value);
          setSelectedIndex(0);
          setDropdownOpen(true);
        }}
        onKeyDown={handleKeyDown}
        onFocus={() => setDropdownOpen(true)}
        placeholder={tokens.length === 0 ? placeholder : undefined}
        className="min-w-[120px] flex-1 bg-transparent py-0.5 text-sm outline-none placeholder:text-muted-foreground/50"
      />
      {showDropdown && (
        <div className="absolute left-0 top-full z-50 mt-1 max-h-64 w-72 overflow-y-auto rounded-lg border border-border bg-popover shadow-lg">
          {results.map((result, idx) => (
            <button
              key={result.item.email}
              type="button"
              onMouseDown={(e) => {
                e.preventDefault();
                selectSuggestion(result.item);
              }}
              className={cn(
                "flex w-full flex-col px-3 py-2 text-left text-sm transition-colors hover:bg-accent",
                idx === selectedIndex && "bg-accent",
              )}
            >
              {result.highlightedName && (
                <span className="font-medium">
                  <HighlightedText html={result.highlightedName} />
                </span>
              )}
              <span className={result.highlightedName ? "text-xs text-muted-foreground" : ""}>
                <HighlightedText html={result.highlightedEmail} />
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
