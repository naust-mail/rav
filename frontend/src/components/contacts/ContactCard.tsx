"use client";

import { Trash2, Building2, Mail, StickyNote, Clock, Tag, Users, Plus, Send, Pencil, Star } from "lucide-react";
import { useState, useRef, useEffect, useMemo } from "react";
import { AnimatePresence } from "framer-motion";
import { useComposeStore } from "@/stores/useComposeStore";
import { useUpdateContact } from "@/hooks/useContacts";
import { ContactDialog } from "@/components/contacts/ContactDialog";
import { Button } from "@/components/ui/button";
import { Chip } from "@/components/ui/Chip";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";
import { createScaleFadeVariants } from "@/lib/motion/variants";
import { useUiStore } from "@/stores/useUiStore";
import type { Contact, ContactGroup } from "@/types/contact";
import {
  useAddGroupMember,
  useRemoveGroupMember,
} from "@/hooks/useContactGroups";
import { useQuery } from "@tanstack/react-query";
import { apiGet } from "@/lib/api";

function formatDate(dateStr: string): string {
  try {
    return new Date(dateStr).toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  } catch {
    return dateStr;
  }
}

function InitialsAvatar({
  name,
  email,
  size = "lg",
}: {
  name: string;
  email: string;
  size?: "sm" | "lg";
}) {
  const letter = (name || email || "?").charAt(0).toUpperCase();

  // Deterministic color from the first character
  const colors = [
    "bg-blue-500",
    "bg-emerald-500",
    "bg-violet-500",
    "bg-amber-500",
    "bg-rose-500",
    "bg-cyan-500",
    "bg-pink-500",
    "bg-teal-500",
    "bg-orange-500",
    "bg-indigo-500",
  ];
  const colorIndex = (name || email).charCodeAt(0) % colors.length;

  return (
    <div
      className={`flex shrink-0 items-center justify-center rounded-full text-white font-semibold ${colors[colorIndex]} ${
        size === "lg" ? "size-14 text-xl" : "size-9 text-sm"
      }`}
    >
      {letter}
    </div>
  );
}

export { InitialsAvatar };

/** Props for the ContactCard component. */
type ContactCardProps = {
  contact: Contact;
  onDelete: (id: string) => void;
  isDeleting: boolean;
  groups?: ContactGroup[];
};

export function ContactCard({
  contact,
  onDelete,
  isDeleting,
  groups = [],
}: ContactCardProps) {
  const addMember = useAddGroupMember();
  const removeMember = useRemoveGroupMember();
  const [groupPickerOpen, setGroupPickerOpen] = useState(false);
  const groupPickerRef = useRef<HTMLDivElement>(null);
  const [deletePopoverOpen, setDeletePopoverOpen] = useState(false);
  const deletePopoverRef = useRef<HTMLDivElement>(null);
  const [editOpen, setEditOpen] = useState(false);
  const [optimisticFavorite, setOptimisticFavorite] = useState<boolean | null>(null);
  const isFavorite = optimisticFavorite ?? contact.is_favorite;
  const updateContact = useUpdateContact();
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const deletePopoverMotion = useMemo(() => createScaleFadeVariants(effectiveAnimationMode), [effectiveAnimationMode]);

  useEffect(() => {
    if (!groupPickerOpen) return;
    function handleClick(e: MouseEvent) {
      if (groupPickerRef.current && !groupPickerRef.current.contains(e.target as Node)) {
        setGroupPickerOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [groupPickerOpen]);

  useEffect(() => {
    if (!deletePopoverOpen) return;
    function handleClick(e: MouseEvent) {
      if (deletePopoverRef.current && !deletePopoverRef.current.contains(e.target as Node)) {
        setDeletePopoverOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [deletePopoverOpen]);

  // Fetch which groups this contact belongs to
  const { data: contactGroupsData } = useQuery({
    queryKey: ["contact-groups-for", contact.id],
    queryFn: async () => {
      // We don't have a direct endpoint, so derive from the groups list
      // by checking membership. For efficiency, we use the group members endpoint.
      // But since that's per-group, let's just use a simple approach:
      // check all groups' member lists. This is fine for small group counts.
      const membershipResults = await Promise.all(
        groups.map(async (g) => {
          try {
            const res = await apiGet<{ members: Contact[] }>(
              `/contact-groups/${g.id}/members`,
            );
            const isMember = res.members.some((m) => m.id === contact.id);
            return { groupId: g.id, isMember };
          } catch {
            return { groupId: g.id, isMember: false };
          }
        }),
      );
      return membershipResults;
    },
    enabled: groups.length > 0,
  });

  const memberGroupIds = new Set(
    contactGroupsData
      ?.filter((r) => r.isMember)
      .map((r) => r.groupId) ?? [],
  );

  const memberGroups = groups.filter((g) => memberGroupIds.has(g.id));
  const nonMemberGroups = groups.filter((g) => !memberGroupIds.has(g.id));

  return (
    <div className="rounded-lg border border-border bg-card p-6">
      <div className="flex items-start gap-4">
        <InitialsAvatar name={contact.name} email={contact.email} size="lg" />

        <div className="min-w-0 flex-1">
          <div className="flex items-start justify-between gap-2">
            <div className="min-w-0">
              {contact.name ? (
                <h2 className="line-clamp-2 text-lg font-semibold text-foreground">{contact.name}</h2>
              ) : (
                <h2 className="break-all text-lg font-medium text-muted-foreground">{contact.email}</h2>
              )}
              {contact.name && (
                <div className="mt-0.5 flex items-start gap-1.5 text-sm text-muted-foreground">
                  <Mail className="mt-0.5 size-3.5 shrink-0" />
                  <span className="break-all">{contact.email}</span>
                </div>
              )}
            </div>
            <div className="flex items-center gap-1">
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={() => {
                  const next = !isFavorite;
                  setOptimisticFavorite(next);
                  updateContact.mutate(
                    {
                      ...contact,
                      company: contact.company || undefined,
                      notes: contact.notes || undefined,
                      is_favorite: next,
                    },
                    { onSettled: () => setOptimisticFavorite(null) },
                  );
                }}
                className={isFavorite ? "text-amber-400 hover:text-amber-500" : "text-muted-foreground hover:text-amber-400"}
                title={isFavorite ? "Remove from favourites" : "Add to favourites"}
              >
                <Star className={isFavorite ? "size-4 fill-current" : "size-4"} />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={() => setEditOpen(true)}
                className="text-muted-foreground"
                title="Edit contact"
              >
                <Pencil className="size-4" />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={() => {
                  useComposeStore.getState().openCompose();
                  useComposeStore.setState({ to: contact.name ? `${contact.name} <${contact.email}>` : contact.email });
                }}
                className="text-muted-foreground"
                title="Compose email"
              >
                <Send className="size-4" />
              </Button>
              <div ref={deletePopoverRef} className="relative">
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="text-muted-foreground hover:text-destructive"
                  title="Delete contact"
                  onClick={() => setDeletePopoverOpen((v) => !v)}
                >
                  <Trash2 className="size-4" />
                </Button>
                <AnimatePresence>
                  {deletePopoverOpen && (
                    <AnimatedDiv
                      variants={deletePopoverMotion}
                      initial="initial"
                      animate="animate"
                      exit="exit"
                      className="absolute right-0 top-full z-20 mt-1 w-52 rounded-lg border border-border bg-background p-3 shadow-lg"
                    >
                      <p className="mb-3 text-sm font-medium text-foreground">Delete this contact?</p>
                      <div className="flex justify-end gap-2">
                        <Button variant="outline" size="sm" onClick={() => setDeletePopoverOpen(false)}>
                          Cancel
                        </Button>
                        <Button
                          variant="destructive"
                          size="sm"
                          disabled={isDeleting}
                          onClick={() => { onDelete(contact.id); setDeletePopoverOpen(false); }}
                        >
                          Delete
                        </Button>
                      </div>
                    </AnimatedDiv>
                  )}
                </AnimatePresence>
              </div>
            </div>
          </div>

          <div className="mt-4 space-y-2">
            {contact.company && (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Building2 className="size-4 shrink-0" />
                <span className="truncate">{contact.company}</span>
              </div>
            )}

            {contact.notes && (
              <div className="flex items-start gap-2 text-sm text-muted-foreground">
                <StickyNote className="size-4 shrink-0 mt-0.5" />
                <span className="line-clamp-4">{contact.notes}</span>
              </div>
            )}

            {/* Groups section */}
            {groups.length > 0 && (
              <div className="flex items-start gap-2 text-sm text-muted-foreground">
                <Users className="size-4 shrink-0 mt-0.5" />
                <div className="flex flex-wrap gap-1">
                  {memberGroups.map((g) => (
                    <Chip
                      key={g.id}
                      onRemove={() => removeMember.mutate({ groupId: g.id, contactId: contact.id })}
                    >
                      <span className="max-w-[120px] truncate">{g.name}</span>
                    </Chip>
                  ))}
                  {nonMemberGroups.length > 0 && (
                    <div ref={groupPickerRef} className="relative">
                      <Chip
                        variant="outline"
                        icon={<Plus className="size-3" />}
                        onClick={() => setGroupPickerOpen((v) => !v)}
                      >
                        Add to group
                      </Chip>
                      {groupPickerOpen && (
                        <div className="absolute left-0 top-full z-10 mt-1 min-w-[140px] max-w-[200px] rounded-md border border-border bg-popover p-1 shadow-md">
                          {nonMemberGroups.map((g) => (
                            <button
                              key={g.id}
                              type="button"
                              onClick={() => {
                                addMember.mutate({ groupId: g.id, contactId: contact.id });
                                setGroupPickerOpen(false);
                              }}
                              className="flex w-full items-center gap-2 rounded-sm px-2 py-1 text-xs text-foreground hover:bg-accent"
                            >
                              <span className="truncate">{g.name}</span>
                            </button>
                          ))}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </div>
            )}

            <div className="flex items-center gap-4 text-xs text-muted-foreground/70">
              <div className="flex items-center gap-1">
                <Clock className="size-3" />
                <span>Created {formatDate(contact.created_at)}</span>
              </div>
              {contact.last_contacted && (
                <span>Last contacted {formatDate(contact.last_contacted)}</span>
              )}
              {contact.contact_count > 0 && (
                <span>{contact.contact_count} interactions</span>
              )}
              <span className="flex items-center gap-0.5">
                <Tag className="size-3" />
                <span className="capitalize">{contact.source}</span>
              </span>
            </div>
          </div>
        </div>
      </div>

      <ContactDialog
        open={editOpen}
        onClose={() => setEditOpen(false)}
        onSubmit={(data) => {
          updateContact.mutate(
            { id: contact.id, source: contact.source, ...data },
            { onSuccess: () => setEditOpen(false) },
          );
        }}
        isPending={updateContact.isPending}
        mode="edit"
        initialEmail={contact.email}
        initialName={contact.name}
        initialCompany={contact.company}
        initialNotes={contact.notes}
      />
    </div>
  );
}
