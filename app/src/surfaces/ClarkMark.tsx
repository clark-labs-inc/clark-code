// The official Clark mark — a single open "C" ring (~70° gap, round caps),
// white on near-black. Geometry from clark/promos/launch-kit/brand/clark-mark.svg.

export function ClarkMark({
  size = 24,
  tile = true,
  className,
}: {
  size?: number;
  tile?: boolean;
  className?: string;
}) {
  return (
    <svg
      viewBox="0 0 1024 1024"
      width={size}
      height={size}
      className={className}
      role="img"
      aria-label="Clark"
    >
      {tile && <rect width="1024" height="1024" rx="224" fill="#0d0d0d" />}
      <path
        d="M 790.5 317 A 340 340 0 1 0 790.5 707"
        stroke="#ffffff"
        strokeWidth="132"
        strokeLinecap="round"
        fill="none"
      />
    </svg>
  );
}
