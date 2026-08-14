import { ChevronLeft, Library } from "lucide-react";
import { ProductExceptionalStateIllustration } from "../../components/ProductExceptionalStateIllustration";
import { productModule } from "../../product/productModule";

export function ArtifactWorkspaceEmpty({ onClose }: { onClose: () => void }) {
  return (
    <section
      aria-label="Artifact workspace"
      className="relative flex min-w-0 flex-1 flex-col bg-bg-elevated"
    >
      <div className="flex h-10 shrink-0 items-stretch border-b border-border-subtle bg-bg-secondary/45">
        <button
          type="button"
          onClick={onClose}
          aria-label="Close artifact workspace"
          title="Close artifact workspace"
          className="grid w-10 shrink-0 place-items-center border-r border-border-subtle text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <ChevronLeft className="size-4" />
        </button>
        <div className="flex min-w-0 items-center px-3 text-xs font-medium text-ink-secondary">
          Artifacts
        </div>
      </div>

      <div className="grid min-h-0 flex-1 place-items-center px-8 py-12 text-center">
        <div className="max-w-xs">
          <ProductExceptionalStateIllustration
            state="empty"
            size={148}
            className="mx-auto"
            label={`${productModule().branding.shortName} is waiting for the first artifact`}
            fallback={(
              <div className="mx-auto grid size-10 place-items-center rounded-xl bg-bg-secondary text-ink-faint ring-1 ring-border-subtle">
                <Library className="size-5" />
              </div>
            )}
          />
          <h1 className="mt-3 font-display text-xl text-ink">No artifacts yet</h1>
          <p className="mt-2 text-sm leading-relaxed text-ink-muted">
            Files and other outputs created in this task will appear here.
          </p>
        </div>
      </div>
    </section>
  );
}
