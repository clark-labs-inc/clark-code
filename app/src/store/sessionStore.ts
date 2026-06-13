import { create } from "zustand";
import { getBridge, type CoreBridge } from "../core-bridge/bridge";
import {
  emptySnapshot,
  type ClientResponse,
  type ProviderInfo,
  type Session,
  type Snapshot,
} from "../core-bridge/types";
import {
  fileToAttachment,
  toUpload,
  MAX_ATTACHMENT_BYTES,
  type PendingAttachment,
} from "../lib/attachments";
import {
  loadAuthSession,
  signInDemo,
  signInWithGoogle,
  signOut as authSignOut,
  type AuthMethod,
  type AuthSession,
} from "../lib/auth";

interface SessionState {
  bridge: CoreBridge | null;
  providers: ProviderInfo[];
  activeProvider: string | null;
  session: Session | null;
  snapshot: Snapshot;
  connecting: boolean;
  error: string | null;
  /** Authenticated user + the Clark connection config it carries. */
  auth: AuthSession | null;
  /** Files staged to send with the next message. */
  attachments: PendingAttachment[];

  init: () => Promise<void>;
  selectProvider: (id: string) => void;
  signIn: (method: AuthMethod) => Promise<void>;
  signOutAuth: () => void;
  startSession: () => Promise<void>;
  endSession: () => void;
  addFiles: (files: File[]) => Promise<void>;
  removeAttachment: (id: string) => void;
  send: (text: string) => Promise<void>;
  cancelActive: () => Promise<void>;
  resolvePermission: (option: string) => Promise<void>;
}

export const useSessionStore = create<SessionState>((set, get) => ({
  bridge: null,
  providers: [],
  activeProvider: null,
  session: null,
  snapshot: emptySnapshot(),
  connecting: false,
  error: null,
  auth: loadAuthSession(),
  attachments: [],

  init: async () => {
    try {
      const bridge = await getBridge();
      const providers = await bridge.listProviders();
      bridge.subscribe((snapshot) => set({ snapshot }));
      set({
        bridge,
        providers,
        activeProvider: providers[0]?.id ?? null,
      });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  selectProvider: (id) => set({ activeProvider: id }),

  signIn: async (method) => {
    const auth = method === "google" ? await signInWithGoogle() : signInDemo();
    set({ auth });
  },

  signOutAuth: () => {
    authSignOut();
    get().endSession();
    set({ auth: null });
  },

  startSession: async () => {
    const { bridge, activeProvider, auth } = get();
    if (!bridge || !activeProvider) return;
    set({ connecting: true, error: null });
    try {
      // The signed-in session carries the Clark connection config (endpoint +
      // token); no credentials are embedded in the app.
      const config = { endpoint: auth?.clark.endpoint, auth_token: auth?.clark.token };
      await bridge.connect(activeProvider, config);
      const session = await bridge.newSession(activeProvider, {});
      set({ session, connecting: false });
    } catch (e) {
      set({ error: String(e), connecting: false });
    }
  },

  endSession: () => {
    for (const a of get().attachments) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
    set({ session: null, snapshot: emptySnapshot(), error: null, attachments: [] });
  },

  addFiles: async (files) => {
    const incoming = files.filter((f) => f.size <= MAX_ATTACHMENT_BYTES);
    const tooBig = files.length - incoming.length;
    if (tooBig > 0) {
      set({ error: `${tooBig} file(s) skipped — over ${MAX_ATTACHMENT_BYTES / 1024 / 1024}MB.` });
    }
    const prepared = await Promise.all(incoming.map(fileToAttachment));
    set((s) => ({ attachments: [...s.attachments, ...prepared] }));
  },

  removeAttachment: (id) => {
    const a = get().attachments.find((x) => x.id === id);
    if (a?.previewUrl) URL.revokeObjectURL(a.previewUrl);
    set((s) => ({ attachments: s.attachments.filter((x) => x.id !== id) }));
  },

  send: async (text) => {
    const { bridge, session, attachments } = get();
    if (!bridge || !session) return;
    if (!text.trim() && attachments.length === 0) return;
    const uploads = attachments.map(toUpload);
    for (const a of attachments) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl);
    set({ attachments: [] });
    await bridge.prompt(session.id, [{ type: "text", text }], uploads);
  },

  cancelActive: async () => {
    const { bridge, session, snapshot } = get();
    if (!bridge || !session) return;
    const runId = Object.values(snapshot.runs).find(
      (r) => r.status === "running" || r.status === "queued",
    )?.id;
    if (runId) await bridge.cancel(session.id, runId);
  },

  resolvePermission: async (option) => {
    const { bridge, session, snapshot } = get();
    if (!bridge || !session || !snapshot.pending_permission) return;
    const response: ClientResponse = {
      kind: "permission",
      request: snapshot.pending_permission.id,
      option,
    };
    await bridge.respond(session.id, response);
  },
}));
