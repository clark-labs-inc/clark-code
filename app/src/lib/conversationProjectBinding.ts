// A cloud conversation may be opened on more than one computer, but its local
// checkout path is not portable. Keep an explicit account-scoped binding on
// this device; never publish it as the conversation's cloud project metadata.
const PREFIX = "clark:conversation-project:";

function key(account: string | null, id: string): string | null {
  return account && id ? `${PREFIX}${encodeURIComponent(account)}:${encodeURIComponent(id)}` : null;
}

export function loadConversationProjectBinding(account: string | null, id: string): string | null {
  const storageKey = key(account, id);
  if (!storageKey) return null;
  try {
    return localStorage.getItem(storageKey)?.trim() || null;
  } catch {
    return null;
  }
}

export function saveConversationProjectBinding(account: string | null, id: string, path: string): void {
  const storageKey = key(account, id);
  if (!storageKey || !path.trim()) return;
  localStorage.setItem(storageKey, path.trim());
}

export function removeConversationProjectBinding(account: string | null, id: string): void {
  const storageKey = key(account, id);
  if (storageKey) localStorage.removeItem(storageKey);
}
