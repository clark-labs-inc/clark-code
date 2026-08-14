import { afterEach, describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import {
  installProductModule,
  neutralProduct,
  type ProductExceptionalStateIllustrationProps,
} from "../product/productModule";
import { ProductExceptionalStateIllustration } from "./ProductExceptionalStateIllustration";

function TestIllustration({ state, label }: ProductExceptionalStateIllustrationProps) {
  return <span aria-label={label}>{state}</span>;
}

describe("ProductExceptionalStateIllustration", () => {
  afterEach(() => installProductModule(neutralProduct));

  it("keeps exceptional-state art owned by the product composition", () => {
    installProductModule({
      ...neutralProduct,
      exceptionalStateIllustration: TestIllustration,
    });

    const markup = renderToStaticMarkup(
      <ProductExceptionalStateIllustration
        state="recovery"
        label="Product character reconnecting"
      />,
    );

    expect(markup).toContain("Product character reconnecting");
    expect(markup).toContain("recovery");
  });

  it("preserves the neutral fallback when a product has no character", () => {
    const markup = renderToStaticMarkup(
      <ProductExceptionalStateIllustration
        state="empty"
        fallback={<span>Neutral empty state</span>}
      />,
    );

    expect(markup).toContain("Neutral empty state");
  });
});
