import { useState } from "react";
import { productName } from "../../product/productModule";
import { AlertTriangle, Check, Loader2, RefreshCw } from "lucide-react";
import { useSessionStore } from "../../store/sessionStore";
import { useAppVersion } from "../../lib/appInfo";
import { cn } from "../../lib/cn";
import { Card, GroupLabel, Row } from "./Primitives";

export function AboutSection() {
  const version = useAppVersion();
  const update = useSessionStore((s) => s.update);
  const updateChecking = useSessionStore((s) => s.updateChecking);
  const updateWaiting = useSessionStore((s) => s.updateWaiting);
  const checkForUpdate = useSessionStore((s) => s.checkForUpdate);
  const applyUpdate = useSessionStore((s) => s.applyUpdate);
  const [checkFeedback, setCheckFeedback] = useState<"up-to-date" | "error" | null>(null);
  const [checkError, setCheckError] = useState<string | null>(null);

  const check = async () => {
    setCheckFeedback(null);
    setCheckError(null);
    const result = await checkForUpdate();
    if (result.status === "up-to-date") setCheckFeedback("up-to-date");
    if (result.status === "error") {
      setCheckFeedback("error");
      setCheckError(result.message);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <GroupLabel>About</GroupLabel>
        <Card>
          <Row name={productName()} sub="Local AI coding agent">
            <span className="font-mono text-sm text-ink-secondary">{version ? `v${version}` : "—"}</span>
          </Row>
        </Card>
      </div>

      <div>
        <GroupLabel>Updates</GroupLabel>
        {update ? (
          <button
            onClick={() => void applyUpdate()}
            disabled={updateWaiting}
            aria-label={`Ready to update ${productName()} to ${update.version}; restart now`}
            className="flex w-full items-center gap-2.5 rounded-lg bg-accent/15 px-3.5 py-2.5 text-sm font-medium text-accent transition hover:bg-accent/25"
          >
            <RefreshCw className={cn("size-4", updateWaiting && "animate-[spin_1.4s_linear_infinite]")} />{" "}
            {updateWaiting
              ? "Finishing active work before updating…"
              : `${productName()} ${update.version} is ready — restart to update`}
          </button>
        ) : (
          <button
            onClick={() => void check()}
            disabled={updateChecking}
            title={checkError ?? undefined}
            className="flex w-full items-center gap-2.5 rounded-lg border border-border-subtle px-3.5 py-2.5 text-sm text-ink-secondary transition hover:bg-bg-hover disabled:opacity-60"
          >
            {updateChecking ? (
              <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" />
            ) : checkFeedback === "up-to-date" ? (
              <Check className="size-4 text-success" />
            ) : checkFeedback === "error" ? (
              <AlertTriangle className="size-4 text-danger" />
            ) : (
              <RefreshCw className="size-4" />
            )}
            {updateChecking
              ? "Checking…"
              : checkFeedback === "up-to-date"
                ? "You're up to date"
                : checkFeedback === "error"
                  ? "Couldn't check — try again"
                  : "Check for updates"}
          </button>
        )}
      </div>
    </div>
  );
}
