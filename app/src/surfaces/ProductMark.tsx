import { productModule, type ProductMarkProps } from "../product/productModule";

export function ProductMark({ size = 24, tile = true, className }: ProductMarkProps) {
  const Mark = productModule().mark;
  if (Mark) return <Mark size={size} tile={tile} className={className} />;
  const initial = productModule().branding.shortName.slice(0, 1).toUpperCase();
  return (
    <span
      aria-label={productModule().branding.name}
      className={`inline-grid place-items-center bg-bg-elevated font-semibold text-ink ${className ?? ""}`}
      style={{ width: size, height: size }}
    >
      {initial}
    </span>
  );
}
