import { KeyRound } from "lucide-react";
import { productModule } from "../product/productModule";
import type { ProductUsageFailureProps } from "../product/productModule";

/** Product-neutral recovery surface for provider access failures. A branded
 * composition may replace the body with its own account action. */
export function UpgradePrompt(props: ProductUsageFailureProps) {
  const product = productModule();
  const ProductFailure = product.usageFailure;
  return (
    <div className="rounded-xl border border-warning/30 bg-warning/10 px-4 py-3">
      <div className="flex items-start gap-3">
        <KeyRound className="mt-0.5 size-4 shrink-0 text-warning" />
        <div className="min-w-0 flex-1">
          {ProductFailure ? <ProductFailure {...props} /> : (
            <>
              <p className="text-sm font-medium text-ink">Provider access required</p>
              <p className="mt-0.5 text-xs leading-relaxed text-ink-secondary">
                This provider cannot start another run. Review its account or
                access settings, then retry. Your local work remains available.
              </p>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
