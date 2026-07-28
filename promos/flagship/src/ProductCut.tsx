import type { ReactNode } from "react";
import {
  AbsoluteFill,
  Easing,
  Img,
  OffthreadVideo,
  Sequence,
  interpolate,
  staticFile,
  useCurrentFrame,
} from "remotion";
import { FPS } from "./Root";

const clamp = {
  extrapolateLeft: "clamp" as const,
  extrapolateRight: "clamp" as const,
};

function Font() {
  return (
    <style>
      {`
        @font-face {
          font-family: "DM Sans";
          src: url("${staticFile("dm-sans.woff2")}") format("woff2");
          font-style: normal;
          font-weight: 100 900;
        }
        * { box-sizing: border-box; }
      `}
    </style>
  );
}

function FullBleed({
  children,
  scale = 1,
  origin = "50% 50%",
}: {
  children: ReactNode;
  scale?: number;
  origin?: string;
}) {
  const frame = useCurrentFrame();
  const drift = interpolate(frame, [0, 180], [scale, scale + 0.025], clamp);
  return (
    <AbsoluteFill style={{ overflow: "hidden", background: "#090909" }}>
      <div
        style={{
          position: "absolute",
          left: 0,
          top: -60,
          width: 1920,
          height: 1200,
          transform: `scale(${drift})`,
          transformOrigin: origin,
        }}
      >
        {children}
      </div>
    </AbsoluteFill>
  );
}

function Source({
  start,
  scale = 1,
  origin,
}: {
  start: number;
  scale?: number;
  origin?: string;
}) {
  return (
    <FullBleed scale={scale} origin={origin}>
      <OffthreadVideo
        muted
        src={staticFile("clark-ui-flagship.mp4")}
        trimBefore={Math.round(start * FPS)}
        style={{ width: "100%", height: "100%", objectFit: "cover" }}
      />
    </FullBleed>
  );
}

function HeldProof() {
  return (
    <FullBleed scale={1.015} origin="66% 54%">
      <Img
        src={staticFile("final-proof.png")}
        style={{ width: "100%", height: "100%", objectFit: "cover" }}
      />
    </FullBleed>
  );
}

function Caption({
  eyebrow,
  children,
  tone = "white",
  align = "left",
}: {
  eyebrow?: string;
  children: ReactNode;
  tone?: "white" | "green" | "red";
  align?: "left" | "right";
}) {
  const frame = useCurrentFrame();
  const p = interpolate(frame, [0, 10], [0, 1], {
    ...clamp,
    easing: Easing.out(Easing.cubic),
  });
  const color = tone === "green" ? "#7be1a8" : tone === "red" ? "#ff8a81" : "#f7f5f1";
  return (
    <div
      style={{
        position: "absolute",
        zIndex: 10,
        left: align === "left" ? 54 : undefined,
        right: align === "right" ? 54 : undefined,
        bottom: 48,
        maxWidth: 870,
        padding: "22px 26px 24px",
        borderRadius: 16,
        background: "rgba(7,7,7,.78)",
        border: "1px solid rgba(255,255,255,.12)",
        boxShadow: "0 18px 60px rgba(0,0,0,.35)",
        color,
        fontFamily: "DM Sans",
        opacity: p,
        transform: `translateY(${interpolate(p, [0, 1], [18, 0])}px)`,
      }}
    >
      {eyebrow && (
        <div
          style={{
            marginBottom: 8,
            color: tone === "white" ? "rgba(255,255,255,.52)" : color,
            fontSize: 15,
            fontWeight: 700,
            letterSpacing: ".12em",
            textTransform: "uppercase",
          }}
        >
          {eyebrow}
        </div>
      )}
      <div style={{ fontSize: 40, fontWeight: 630, lineHeight: 1.05, letterSpacing: "-.035em" }}>
        {children}
      </div>
    </div>
  );
}

function ProofOpen() {
  const frame = useCurrentFrame();
  const p = interpolate(frame, [0, 14], [0, 1], {
    ...clamp,
    easing: Easing.out(Easing.cubic),
  });
  return (
    <AbsoluteFill>
      <HeldProof />
      <div
        style={{
          position: "absolute",
          inset: 0,
          background: "linear-gradient(90deg, rgba(0,0,0,.94) 0%, rgba(0,0,0,.62) 34%, transparent 68%)",
        }}
      />
      <div
        style={{
          position: "absolute",
          left: 72,
          top: 300,
          color: "#f7f5f1",
          fontFamily: "DM Sans",
          opacity: p,
          transform: `translateY(${interpolate(p, [0, 1], [18, 0])}px)`,
        }}
      >
        <div style={{ color: "#7be1a8", fontSize: 17, fontWeight: 750, letterSpacing: ".12em" }}>
          VERIFIED LOCAL + RUNNER
        </div>
        <div style={{ marginTop: 14, fontSize: 82, fontWeight: 680, lineHeight: .92, letterSpacing: "-.065em" }}>
          14 passed.
          <br />
          <span style={{ color: "#7be1a8" }}>0 failed.</span>
        </div>
        <div style={{ marginTop: 24, maxWidth: 540, fontSize: 25, lineHeight: 1.35, color: "rgba(255,255,255,.68)" }}>
          A Windows-only reconnect failure, fixed on both machines.
        </div>
      </div>
    </AbsoluteFill>
  );
}

function End() {
  const frame = useCurrentFrame();
  const p = interpolate(frame, [0, 12], [0, 1], {
    ...clamp,
    easing: Easing.out(Easing.cubic),
  });
  return (
    <AbsoluteFill style={{ background: "#080808", display: "grid", placeItems: "center" }}>
      <div
        style={{
          color: "#f7f5f1",
          fontFamily: "DM Sans",
          textAlign: "center",
          opacity: p,
          transform: `scale(${interpolate(p, [0, 1], [.98, 1])})`,
        }}
      >
        <Img src={staticFile("icon.png")} style={{ width: 70, height: 70, borderRadius: 18 }} />
        <div style={{ marginTop: 20, fontSize: 56, fontWeight: 680, letterSpacing: "-.05em" }}>Clark Code</div>
        <div style={{ marginTop: 8, color: "rgba(255,255,255,.62)", fontSize: 24 }}>
          Agent work you can see.
        </div>
        <div style={{ marginTop: 26, color: "#9b89ff", fontSize: 19, fontWeight: 650 }}>
          clarkchat.com/clark-code
        </div>
      </div>
    </AbsoluteFill>
  );
}

export function ProductCut() {
  return (
    <AbsoluteFill style={{ background: "#090909" }}>
      <Font />

      <Sequence from={0} durationInFrames={75}>
        <ProofOpen />
      </Sequence>

      <Sequence from={75} durationInFrames={105}>
        <Source start={2.2} scale={1.05} origin="64% 78%" />
        <Caption eyebrow="The report">Reconnect spins forever after sleep on Windows.</Caption>
      </Sequence>

      <Sequence from={180} durationInFrames={120}>
        <Source start={6.2} scale={1.06} origin="65% 52%" />
        <Caption eyebrow="Investigate">Three bounded threads. One question.</Caption>
      </Sequence>

      <Sequence from={300} durationInFrames={120}>
        <Source start={9.0} scale={1.08} origin="66% 34%" />
        <Caption eyebrow="The hard case" tone="red">First theory: wrong.</Caption>
      </Sequence>

      <Sequence from={420} durationInFrames={135}>
        <Source start={13.0} scale={1.07} origin="66% 62%" />
        <Caption eyebrow="Root cause">The cursor carried a hidden Windows carriage return.</Caption>
      </Sequence>

      <Sequence from={555} durationInFrames={135}>
        <Source start={17.5} scale={1.07} origin="66% 68%" />
        <Caption eyebrow="Scoped change">One line, at the first broken boundary.</Caption>
      </Sequence>

      <Sequence from={690} durationInFrames={120}>
        <Source start={22.0} scale={1.08} origin="66% 70%" />
        <Caption eyebrow="Proof" tone="green">Local passed. Windows runner passed.</Caption>
      </Sequence>

      <Sequence from={810} durationInFrames={45}>
        <HeldProof />
        <Caption eyebrow="Verified" tone="green">14 passed. 0 failed.</Caption>
      </Sequence>

      <Sequence from={855} durationInFrames={45}>
        <End />
      </Sequence>
    </AbsoluteFill>
  );
}
