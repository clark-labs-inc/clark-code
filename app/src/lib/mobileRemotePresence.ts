export const MOBILE_REMOTE_HEARTBEAT_INTERVAL_MS = 30_000;

type RefreshPresence = () => Promise<void> | void;

/**
 * Publish the time-sensitive host lease before starting optional repository
 * refresh work. Repository discovery can take tens of seconds on a large
 * workspace, so it must never sit in front of the presence request.
 */
export async function publishMobileRemotePresence(
  publish: RefreshPresence,
  refreshRepositories?: RefreshPresence,
): Promise<void> {
  await publish();
  if (refreshRepositories) {
    void Promise.resolve()
      .then(() => refreshRepositories())
      .catch(() => undefined);
  }
}

/** Keep host presence on its own clock, independent of command long-polling. */
export class MobileRemotePresenceLoop {
  private stopped = true;
  private inFlight = false;
  private pending = false;
  private timer: ReturnType<typeof setInterval> | null = null;

  constructor(private readonly refresh: RefreshPresence) {}

  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    this.requestRefresh();
    this.timer = setInterval(
      () => this.requestRefresh(),
      MOBILE_REMOTE_HEARTBEAT_INTERVAL_MS,
    );
  }

  refreshNow(): void {
    this.requestRefresh();
  }

  stop(): void {
    this.stopped = true;
    this.pending = false;
    if (this.timer !== null) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }

  private requestRefresh(): void {
    if (this.stopped) return;
    if (this.inFlight) {
      this.pending = true;
      return;
    }
    this.inFlight = true;
    Promise.resolve()
      .then(() => this.refresh())
      .catch(() => undefined)
      .finally(() => {
        this.inFlight = false;
        if (this.pending && !this.stopped) {
          this.pending = false;
          this.requestRefresh();
        }
      });
  }
}
