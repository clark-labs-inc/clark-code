import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { AnimatePresence, motion } from "motion/react";
import { ArrowUp, Square, Paperclip, X, FileText } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { useFileDrop, usePaste } from "../lib/attachmentSources";
import { prettySize } from "../lib/attachments";
import { cn } from "../lib/cn";

function AttachmentChips() {
  const attachments = useSessionStore((s) => s.attachments);
  const remove = useSessionStore((s) => s.removeAttachment);
  if (attachments.length === 0) return null;
  return (
    <div className="mb-2 flex flex-wrap gap-2">
      <AnimatePresence initial={false}>
        {attachments.map((a) => (
          <motion.div
            key={a.id}
            layout
            initial={{ opacity: 0, scale: 0.9 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.9 }}
            transition={{ duration: 0.15 }}
            className="group relative flex items-center gap-2 rounded-lg border border-border bg-bg-elevated py-1 pl-1 pr-2"
          >
            {a.previewUrl ? (
              <img src={a.previewUrl} alt="" className="size-8 rounded-md object-cover" />
            ) : (
              <span className="grid size-8 place-items-center rounded-md bg-bg-tertiary text-ink-muted">
                <FileText className="size-4" />
              </span>
            )}
            <span className="max-w-40 truncate text-xs text-ink-secondary">{a.filename}</span>
            <span className="text-[0.7rem] text-ink-faint">{prettySize(a.size)}</span>
            <button
              onClick={() => remove(a.id)}
              aria-label={`Remove ${a.filename}`}
              className="grid size-4 place-items-center rounded-full bg-ink/10 text-ink-muted transition hover:bg-danger/20 hover:text-danger"
            >
              <X className="size-3" />
            </button>
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}

export function Composer() {
  const [value, setValue] = useState("");
  const taRef = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const session = useSessionStore((s) => s.session);
  const send = useSessionStore((s) => s.send);
  const cancelActive = useSessionStore((s) => s.cancelActive);
  const runs = useSessionStore((s) => s.snapshot.runs);
  const attachments = useSessionStore((s) => s.attachments);
  const addFiles = useSessionStore((s) => s.addFiles);

  const busy = Object.values(runs).some((r) => r.status === "running" || r.status === "queued");
  const { dragging, handlers } = useFileDrop((files) => void addFiles(files));
  usePaste((files) => void addFiles(files), !!session);

  useEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "0px";
    ta.style.height = Math.min(ta.scrollHeight, 200) + "px";
  }, [value]);

  const canSend = !!session && (value.trim().length > 0 || attachments.length > 0);

  const submit = async () => {
    if (!canSend) return;
    const t = value;
    setValue("");
    await send(t.trim());
  };

  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit();
    }
  };

  return (
    <div className="border-t border-border bg-bg px-5 py-3.5" {...handlers}>
      <div
        className={cn(
          "relative mx-auto max-w-3xl rounded-xl border bg-bg-elevated px-3 py-2 shadow-sm transition-colors",
          dragging ? "border-accent/60 ring-2 ring-accent/15" : "border-border focus-within:border-accent/40",
        )}
      >
        {dragging && (
          <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center rounded-xl bg-bg-elevated/90 text-sm font-medium text-accent">
            Drop files to attach
          </div>
        )}

        <AttachmentChips />

        <div className="flex items-end gap-2">
          <input
            ref={fileRef}
            type="file"
            multiple
            hidden
            onChange={(e) => {
              const files = Array.from(e.target.files ?? []);
              if (files.length) void addFiles(files);
              e.target.value = "";
            }}
          />
          <button
            onClick={() => fileRef.current?.click()}
            disabled={!session}
            aria-label="Attach files"
            className="grid size-8 shrink-0 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover disabled:opacity-40"
          >
            <Paperclip className="size-4" />
          </button>

          <textarea
            ref={taRef}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={onKey}
            rows={1}
            aria-label="Message Clark"
            placeholder={session ? "Message Clark…  (paste or drop files to attach)" : "Start a session first"}
            disabled={!session}
            className="max-h-52 flex-1 resize-none bg-transparent py-1 text-sm leading-relaxed text-ink outline-none placeholder:text-ink-faint disabled:opacity-50"
          />

          {busy ? (
            <button
              onClick={() => void cancelActive()}
              aria-label="Stop"
              className="grid size-8 shrink-0 place-items-center rounded-lg bg-danger/12 text-danger transition hover:bg-danger/20"
            >
              <Square className="size-3.5 fill-current" />
            </button>
          ) : (
            <button
              onClick={() => void submit()}
              disabled={!canSend}
              aria-label="Send"
              className="grid size-8 shrink-0 place-items-center rounded-lg bg-accent text-on-accent transition hover:bg-accent-hover disabled:opacity-35"
            >
              <ArrowUp className="size-4" />
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
