import type { ReactNode } from "react";
import type { ProductExceptionalStateIllustrationProps } from "../product/productModule";
import { productModule } from "../product/productModule";

export function ProductExceptionalStateIllustration({
  fallback = null,
  ...props
}: ProductExceptionalStateIllustrationProps & { fallback?: ReactNode }) {
  const Illustration = productModule().exceptionalStateIllustration;
  return Illustration ? <Illustration {...props} /> : fallback;
}
