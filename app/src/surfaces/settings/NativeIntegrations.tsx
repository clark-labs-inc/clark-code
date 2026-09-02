import { useEffect, useState } from "react";
import { MessageSquare, RefreshCw } from "lucide-react";
import { useSessionStore } from "../../store/sessionStore";
import {
  integrationRequest,
  type IntegrationAvailability,
  type IntegrationConversation,
  type IntegrationManifest,
  type IntegrationMessage,
} from "../../lib/integrations";
import { Card, GroupLabel } from "./Primitives";

const button =
  "rounded-lg border border-border-subtle px-3 py-2 text-xs font-medium text-ink-secondary hover:bg-bg-hover disabled:opacity-40";

/** Catalog discovery stays generic; each compiled adapter owns its bounded UI. */
export function NativeIntegrations() {
  const [catalog, setCatalog] = useState<IntegrationManifest[]>([]);
  const [error, setError] = useState("");
  const task = useSessionStore((state) => state.session?.id ?? null);

  useEffect(() => {
    let active = true;
    void integrationRequest<IntegrationManifest[]>({ action: "catalog" })
      .then((items) => {
        if (active) setCatalog(items);
      })
      .catch((cause) => {
        if (active) setError(String(cause));
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <div>
      <GroupLabel>Native integrations</GroupLabel>
      {error && <p className="text-sm text-ink-muted">{error}</p>}
      <Card>
        {catalog.map((manifest) => (
          <IntegrationPanel
            key={`${task}:${manifest.id}`}
            manifest={manifest}
            task={task}
          />
        ))}
      </Card>
    </div>
  );
}

function IntegrationPanel({
  manifest,
  task,
}: {
  manifest: IntegrationManifest;
  task: string | null;
}) {
  const [status, setStatus] = useState<IntegrationAvailability | null>(null);
  const [conversations, setConversations] =
    useState<IntegrationConversation[] | null>(null);
  const [conversation, setConversation] = useState("");
  const [messages, setMessages] = useState<IntegrationMessage[]>([]);
  const [selected, setSelected] = useState(new Set<string>());
  const [enabledCount, setEnabledCount] = useState<number | null>(null);
  const [error, setError] = useState("");
  const [working, setWorking] = useState(false);
  const id = manifest.id;

  function clear() {
    setConversations(null);
    setConversation("");
    setMessages([]);
    setSelected(new Set());
    setEnabledCount(null);
  }

  useEffect(() => {
    let active = true;
    void integrationRequest<IntegrationAvailability>({ action: "status", id })
      .then((next) => {
        if (active) setStatus(next);
      })
      .catch((cause) => {
        if (active) setError(String(cause));
      });
    return () => {
      active = false;
    };
  }, [id]);

  useEffect(() => {
    const revokeWhenHidden = () => {
      if (!document.hidden) return;
      clear();
      void integrationRequest({ action: "revoke", id }).catch(() => {});
    };
    document.addEventListener("visibilitychange", revokeWhenHidden);
    return () =>
      document.removeEventListener("visibilitychange", revokeWhenHidden);
  }, [id]);

  async function run(operation: () => Promise<void>) {
    if (working) return;
    setWorking(true);
    setError("");
    try {
      await operation();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setWorking(false);
    }
  }

  function changeSelection(messageId: string, checked: boolean) {
    const next = new Set(selected);
    if (checked) next.add(messageId);
    else next.delete(messageId);
    setSelected(next);
    setEnabledCount(null);
    if (task) {
      void run(async () => {
        await integrationRequest({ action: "disable_read_tool", id }, task);
      });
    }
  }

  return (
    <section className="space-y-4 py-3" aria-label={`${manifest.name} integration`}>
      <div className="flex items-center gap-2">
        <MessageSquare className="size-4 text-ink-muted" />
        <h4 className="text-sm font-medium text-ink">{manifest.name}</h4>
        {manifest.experimental && (
          <span className="text-xs text-warning">
            Read-only prototype · isolation incomplete
          </span>
        )}
      </div>
      <p className="text-xs leading-relaxed text-ink-muted">
        {manifest.description}
      </p>
      <div className="rounded-lg border border-warning/30 bg-warning/5 p-3 text-xs leading-relaxed text-ink-secondary">
        {status?.detail ?? "Checking native availability…"}
      </div>
      {!task && (
        <p className="text-xs text-ink-muted">
          Open a task before connecting. There is no global task access.
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <button
          className={button}
          disabled={working || !task || !status?.supported}
          onClick={() =>
            void run(async () => {
              clear();
              setConversations(
                await integrationRequest<IntegrationConversation[]>(
                  { action: "connect", id },
                  task,
                ),
              );
            })
          }
        >
          {conversations
            ? "Reconnect read access"
            : "Connect read access for this task…"}
        </button>
        <button
          className={button}
          disabled={working || !status?.supported}
          onClick={() =>
            void run(async () => {
              await integrationRequest({ action: "open_settings" });
            })
          }
        >
          Full Disk Access settings
        </button>
        <button
          className={button}
          disabled={working}
          onClick={() =>
            void run(async () => {
              await integrationRequest({ action: "revoke", id });
              clear();
            })
          }
        >
          Revoke task access
        </button>
      </div>
      <p className="text-xs text-ink-faint">
        Revoking here clears Clark's task grant and tool selection. Remove Full
        Disk Access separately to revoke the macOS app permission.
      </p>

      {conversations && (
        <>
          <label className="block space-y-1 text-xs text-ink-secondary">
            <span>Choose one self-conversation</span>
            <select
              aria-label="Self-conversation"
              className="w-full rounded-lg bg-bg-secondary p-2"
              value={conversation}
              disabled={working}
              onChange={(event) => {
                const value = event.target.value;
                setConversation("");
                setMessages([]);
                setSelected(new Set());
                setEnabledCount(null);
                if (!value) return;
                void run(async () => {
                  const result = await integrationRequest<IntegrationMessage[]>(
                    { action: "select", id, conversation_id: value },
                    task,
                  );
                  setMessages(result);
                  setConversation(value);
                });
              }}
            >
              <option value="">Select…</option>
              {conversations.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.self_address}
                </option>
              ))}
            </select>
          </label>
          {conversations.length === 0 && (
            <p className="text-xs text-ink-muted">
              No eligible self-conversation was found. Create a text
              conversation with your own iMessage address first. Group chats,
              SMS, and arbitrary recipients are excluded.
            </p>
          )}
        </>
      )}

      {conversation && (
        <>
          <div className="flex items-center justify-between gap-2">
            <p className="text-xs text-ink-muted">
              Choose 1–20 of the 50 most recent plain-text messages.
            </p>
            <button
              className={button}
              aria-label="Refresh selected conversation"
              disabled={working}
              onClick={() =>
                void run(async () => {
                  setSelected(new Set());
                  setEnabledCount(null);
                  setMessages(
                    await integrationRequest<IntegrationMessage[]>(
                      { action: "select", id, conversation_id: conversation },
                      task,
                    ),
                  );
                })
              }
            >
              <RefreshCw className="size-3.5" />
            </button>
          </div>
          <div className="max-h-64 space-y-2 overflow-y-auto rounded-lg border border-border-subtle p-3">
            {messages.map((message) => (
              <label
                key={message.id}
                className="flex items-start gap-2 text-xs text-ink-secondary"
              >
                <input
                  type="checkbox"
                  checked={selected.has(message.id)}
                  disabled={
                    working ||
                    (!selected.has(message.id) && selected.size >= 20)
                  }
                  onChange={(event) =>
                    changeSelection(message.id, event.target.checked)
                  }
                />
                <span className="min-w-0 whitespace-pre-wrap break-words">
                  {message.text}
                </span>
              </label>
            ))}
            {messages.length === 0 && (
              <p className="text-xs text-ink-muted">No text found.</p>
            )}
          </div>
          <button
            className={button}
            disabled={working || selected.size === 0}
            onClick={() =>
              void run(async () => {
                const count = await integrationRequest<number>(
                  {
                    action: "enable_read_tool",
                    id,
                    message_ids: [...selected],
                  },
                  task,
                );
                setEnabledCount(count);
              })
            }
          >
            Enable selected text for this task’s read tool
          </button>
          <p className="text-xs text-ink-faint">
            This enables only <code>read_imessage_selection</code>. It accepts
            no arguments and cannot choose another conversation or future
            message. When called, its result is shared with this task’s model as
            untrusted quoted context.
          </p>
          {enabledCount !== null && (
            <p role="status" className="text-xs text-ink-secondary">
              Read tool enabled for {enabledCount} exact message
              {enabledCount === 1 ? "" : "s"}. Changing the selection disables
              it until you enable again.
            </p>
          )}
        </>
      )}
      {working && (
        <p role="status" className="text-xs text-ink-muted">
          Waiting for the native operation…
        </p>
      )}
      {error && (
        <p role="alert" className="text-xs text-danger">
          {error}
        </p>
      )}
    </section>
  );
}
