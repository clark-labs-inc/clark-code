import { Component, type ErrorInfo, type ReactNode } from "react";
import { captureFrontendException } from "../lib/localCapture";
import { productModule } from "../product/productModule";
import { ProductExceptionalStateIllustration } from "./ProductExceptionalStateIllustration";

interface Props {
  title?: string;
  resetKey?: string | number | null;
  onDismiss?: () => void;
  children: ReactNode;
}

interface State {
  error: Error | null;
  reference: string | null;
}

export function privacySafePanelReference(error: Error, componentStack = ""): string {
  const source = `${error.name}\n${error.message}\n${componentStack}`;
  let hash = 0x811c9dc5;
  for (let index = 0; index < source.length; index += 1) {
    hash ^= source.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return `DESKTOP-${(hash >>> 0).toString(16).toUpperCase().padStart(8, "0")}`;
}

/** Contains one render failure without exposing raw provider/runtime details or
 * blanking the rest of the workspace. The reference is safe to include in a
 * support report; raw exception text stays inside React's catch boundary. */
export class PanelErrorBoundary extends Component<Props, State> {
  state: State = { error: null, reference: null };

  static getDerivedStateFromError(error: Error): State {
    return { error, reference: null };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    const reference = privacySafePanelReference(error, info.componentStack ?? "");
    this.setState({ reference });
    captureFrontendException(error, {
      kind: "boundary",
      reference,
      componentStack: info.componentStack ?? undefined,
    });
    console.error(`[PanelErrorBoundary] ${reference}`);
  }

  componentDidUpdate(previous: Props) {
    if (this.state.error && previous.resetKey !== this.props.resetKey) {
      this.reset();
    }
  }

  private reset = () => this.setState({ error: null, reference: null });

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="grid min-h-40 min-w-0 flex-1 place-items-center bg-bg p-5">
        <div className="max-w-sm rounded-xl border border-danger/25 bg-danger/5 p-5 text-center">
          <ProductExceptionalStateIllustration
            state="recovery"
            size={96}
            label={`${productModule().branding.shortName} is reconnecting this panel`}
            className="mx-auto mb-2"
          />
          <h2 className="text-sm font-semibold text-ink">
            {this.props.title ?? "This panel needs to restart"}
          </h2>
          <p className="mt-2 text-xs leading-5 text-ink-muted">
            Your conversation is still saved. Retry this panel without restarting the rest of the agent.
          </p>
          {this.state.reference && (
            <p className="mt-2 text-xs text-ink-faint">Reference {this.state.reference}</p>
          )}
          <div className="mt-4 flex justify-center gap-2">
            {this.props.onDismiss && (
              <button
                type="button"
                onClick={this.props.onDismiss}
                className="rounded-lg border border-border px-3 py-1.5 text-xs text-ink-secondary hover:bg-bg-hover"
              >
                Close
              </button>
            )}
            <button
              type="button"
              onClick={this.reset}
              className="rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-on-accent hover:bg-accent-hover"
            >
              Retry
            </button>
          </div>
        </div>
      </div>
    );
  }
}
