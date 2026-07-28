import type { CSSProperties, ReactNode } from "react";
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

const colors = {
  ink: "#f6f3ed",
  muted: "#aaa6a0",
  paper: "#e9e4da",
  black: "#070706",
  panel: "#11110f",
  accent: "#8e79ff",
  green: "#6fca98",
  red: "#ef766f",
};

const clamp = {
  extrapolateLeft: "clamp" as const,
  extrapolateRight: "clamp" as const,
};

const enter = (frame: number, start = 0, duration = 16) =>
  interpolate(frame, [start, start + duration], [0, 1], {
    ...clamp,
    easing: Easing.out(Easing.cubic),
  });

function Fonts() {
  return (
    <style>
      {`
        @font-face {
          font-family: "DM Sans";
          src: url("${staticFile("dm-sans.woff2")}") format("woff2");
          font-style: normal;
          font-weight: 100 900;
        }
        @font-face {
          font-family: "Newsreader";
          src: url("${staticFile("newsreader.woff2")}") format("woff2");
          font-style: normal;
          font-weight: 200 800;
        }
        @font-face {
          font-family: "JetBrains Mono";
          src: url("${staticFile("jetbrains-mono.woff2")}") format("woff2");
          font-style: normal;
          font-weight: 500;
        }
        * { box-sizing: border-box; }
      `}
    </style>
  );
}

function Atmosphere() {
  const frame = useCurrentFrame();
  const x = interpolate(frame, [0, 1260], [12, 88], clamp);
  return (
    <AbsoluteFill
      style={{
        overflow: "hidden",
        background:
          "radial-gradient(circle at 15% 15%, rgba(142,121,255,.11), transparent 34%), #070706",
      }}
    >
      <div
        style={{
          position: "absolute",
          inset: "-25%",
          opacity: 0.34,
          background: `radial-gradient(circle at ${x}% 48%, rgba(142,121,255,.25), transparent 22%)`,
        }}
      />
      <div
        style={{
          position: "absolute",
          inset: 0,
          opacity: 0.12,
          backgroundImage:
            "linear-gradient(rgba(255,255,255,.045) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,.045) 1px, transparent 1px)",
          backgroundSize: "64px 64px",
          maskImage: "linear-gradient(to bottom, black, transparent 72%)",
        }}
      />
      <div
        style={{
          position: "absolute",
          inset: 0,
          opacity: 0.1,
          backgroundImage:
            "url(\"data:image/svg+xml,%3Csvg viewBox='0 0 180 180' xmlns='http://www.w3.org/2000/svg'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='.82' numOctaves='4' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)' opacity='.38'/%3E%3C/svg%3E\")",
          mixBlendMode: "soft-light",
        }}
      />
    </AbsoluteFill>
  );
}

function Brand({ light = false }: { light?: boolean }) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        color: light ? colors.black : colors.ink,
        fontFamily: "DM Sans",
        fontSize: 22,
        fontWeight: 650,
        letterSpacing: "-0.02em",
      }}
    >
      <Img src={staticFile("icon.png")} style={{ width: 38, height: 38, borderRadius: 10 }} />
      Clark Code
    </div>
  );
}

function Kicker({ children, tone = "accent" }: { children: ReactNode; tone?: "accent" | "red" | "green" }) {
  const toneColor = tone === "green" ? colors.green : tone === "red" ? colors.red : colors.accent;
  return (
    <div
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 10,
        fontFamily: "JetBrains Mono",
        fontSize: 17,
        letterSpacing: "0.12em",
        textTransform: "uppercase",
        color: toneColor,
      }}
    >
      <span
        style={{
          width: 8,
          height: 8,
          borderRadius: 99,
          background: toneColor,
          boxShadow: `0 0 22px ${toneColor}`,
        }}
      />
      {children}
    </div>
  );
}

function ProofChip({ children, tone = "green" }: { children: ReactNode; tone?: "green" | "neutral" }) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 10,
        minHeight: 42,
        padding: "0 16px",
        borderRadius: 999,
        border: `1px solid ${tone === "green" ? "rgba(111,202,152,.35)" : "rgba(255,255,255,.14)"}`,
        background: tone === "green" ? "rgba(111,202,152,.10)" : "rgba(255,255,255,.06)",
        color: tone === "green" ? colors.green : colors.paper,
        fontFamily: "JetBrains Mono",
        fontSize: 16,
      }}
    >
      <span style={{ fontSize: 18 }}>{tone === "green" ? "✓" : "•"}</span>
      {children}
    </div>
  );
}

function Window({
  children,
  style,
  title = "northstar-desktop  /  Fix the reconnect regression",
}: {
  children: ReactNode;
  style?: CSSProperties;
  title?: string;
}) {
  return (
    <div
      style={{
        position: "relative",
        overflow: "hidden",
        borderRadius: 22,
        border: "1px solid rgba(255,255,255,.16)",
        background: "#090908",
        boxShadow: "0 42px 120px rgba(0,0,0,.58), 0 0 0 1px rgba(142,121,255,.08)",
        ...style,
      }}
    >
      <div
        style={{
          height: 46,
          display: "flex",
          alignItems: "center",
          gap: 9,
          padding: "0 16px",
          borderBottom: "1px solid rgba(255,255,255,.08)",
          background: "rgba(20,20,18,.96)",
        }}
      >
        {["#ff6b64", "#f5bd4f", "#5bcf66"].map((color) => (
          <span key={color} style={{ width: 11, height: 11, borderRadius: 99, background: color, opacity: 0.82 }} />
        ))}
        <span
          style={{
            marginLeft: 12,
            color: "rgba(255,255,255,.48)",
            fontFamily: "JetBrains Mono",
            fontSize: 13,
          }}
        >
          {title}
        </span>
      </div>
      <div style={{ position: "absolute", inset: "46px 0 0" }}>{children}</div>
    </div>
  );
}

function ScreenImage({ src, zoom = 1 }: { src: string; zoom?: number }) {
  return (
    <Img
      src={staticFile(src)}
      style={{
        width: "100%",
        height: "100%",
        objectFit: "cover",
        objectPosition: "center",
        transform: `scale(${zoom})`,
      }}
    />
  );
}

function ProofOpen() {
  const frame = useCurrentFrame();
  const p = enter(frame, 0, 24);
  return (
    <AbsoluteFill style={{ padding: "72px 84px 62px", color: colors.ink }}>
      <div style={{ opacity: p, transform: `translateY(${interpolate(p, [0, 1], [24, 0])}px)` }}>
        <Brand />
      </div>
      <div
        style={{
          position: "absolute",
          left: 84,
          top: 200,
          width: 630,
          opacity: enter(frame, 8, 22),
          transform: `translateY(${interpolate(enter(frame, 8, 22), [0, 1], [34, 0])}px)`,
        }}
      >
        <Kicker tone="green">Verified on both machines</Kicker>
        <h1
          style={{
            margin: "28px 0 22px",
            fontFamily: "Newsreader",
            fontSize: 102,
            fontWeight: 430,
            lineHeight: 0.9,
            letterSpacing: "-0.055em",
          }}
        >
          14 passed.
          <br />
          <em style={{ color: colors.green, fontWeight: 420 }}>0 failed.</em>
        </h1>
        <p
          style={{
            margin: 0,
            maxWidth: 560,
            color: colors.muted,
            fontFamily: "DM Sans",
            fontSize: 26,
            lineHeight: 1.35,
          }}
        >
          A platform-specific reconnect bug, diagnosed and proven—not just patched.
        </p>
        <div style={{ display: "flex", gap: 12, marginTop: 34 }}>
          <ProofChip>Local</ProofChip>
          <ProofChip>Windows runner</ProofChip>
        </div>
      </div>
      <Window
        style={{
          position: "absolute",
          width: 1030,
          height: 644,
          right: -54,
          top: 180,
          opacity: enter(frame, 14, 24),
          transform: `perspective(1400px) rotateY(-6deg) rotateX(2deg) translateX(${interpolate(
            enter(frame, 14, 24),
            [0, 1],
            [90, 0],
          )}px) scale(1.03)`,
        }}
      >
        <ScreenImage src="final-proof.png" zoom={1.02} />
      </Window>
    </AbsoluteFill>
  );
}

function Problem() {
  const frame = useCurrentFrame();
  const p = enter(frame, 0, 18);
  return (
    <AbsoluteFill style={{ padding: "76px 92px", color: colors.ink }}>
      <div style={{ opacity: p }}><Brand /></div>
      <div
        style={{
          position: "absolute",
          left: 92,
          top: 238,
          width: 740,
          opacity: enter(frame, 8, 20),
        }}
      >
        <Kicker tone="red">This was the report</Kicker>
        <h2
          style={{
            margin: "28px 0 26px",
            fontFamily: "Newsreader",
            fontWeight: 430,
            fontSize: 72,
            lineHeight: 1.02,
            letterSpacing: "-0.045em",
          }}
        >
          “Reconnect spins forever after sleep on Windows.”
        </h2>
        <div
          style={{
            display: "grid",
            gap: 10,
            fontFamily: "JetBrains Mono",
            fontSize: 17,
            color: "#b8b2aa",
          }}
        >
          <span>08:41:17 resume cursor=evt_932\r\n</span>
          <span style={{ color: colors.red }}>08:41:17 rejected cursor: invalid identifier</span>
          <span>08:41:18 reconnect attempt=7 delay=1000ms</span>
        </div>
      </div>
      <Window
        style={{
          position: "absolute",
          width: 880,
          height: 550,
          right: 86,
          top: 206,
          opacity: enter(frame, 12, 22),
          transform: `translateX(${interpolate(enter(frame, 12, 22), [0, 1], [70, 0])}px)`,
        }}
      >
        <ScreenImage src="problem.png" zoom={1.03} />
      </Window>
      <div
        style={{
          position: "absolute",
          left: 92,
          bottom: 74,
          color: colors.muted,
          fontFamily: "DM Sans",
          fontSize: 22,
          opacity: enter(frame, 32, 16),
        }}
      >
        No clean reproduction. No obvious cause. Release blocked.
      </div>
    </AbsoluteFill>
  );
}

function ScreenRun({
  sourceStart,
  label,
  headline,
  tone = "accent",
  camera = "center",
}: {
  sourceStart: number;
  label: string;
  headline: string;
  tone?: "accent" | "red" | "green";
  camera?: "center" | "top" | "bottom";
}) {
  const frame = useCurrentFrame();
  const p = enter(frame, 0, 18);
  const y = camera === "top" ? "42%" : camera === "bottom" ? "58%" : "50%";
  return (
    <AbsoluteFill style={{ padding: "52px 66px 58px", color: colors.ink }}>
      <div
        style={{
          display: "flex",
          alignItems: "flex-end",
          justifyContent: "space-between",
          height: 112,
          padding: "0 10px 24px",
          opacity: p,
        }}
      >
        <div>
          <Kicker tone={tone}>{label}</Kicker>
          <div
            style={{
              marginTop: 11,
              fontFamily: "Newsreader",
              fontSize: 43,
              lineHeight: 1,
              letterSpacing: "-0.035em",
            }}
          >
            {headline}
          </div>
        </div>
        <Brand />
      </div>
      <Window
        style={{
          position: "absolute",
          left: 66,
          right: 66,
          top: 164,
          bottom: 58,
          opacity: p,
          transform: `translateY(${interpolate(p, [0, 1], [32, 0])}px) scale(${interpolate(
            frame,
            [0, 210],
            [1.012, 1.035],
            clamp,
          )})`,
          transformOrigin: `50% ${y}`,
        }}
      >
        <OffthreadVideo
          muted
          src={staticFile("clark-ui-flagship.mp4")}
          trimBefore={Math.round(sourceStart * FPS)}
          style={{ width: "100%", height: "100%", objectFit: "cover" }}
        />
      </Window>
    </AbsoluteFill>
  );
}

function WrongTheory() {
  const frame = useCurrentFrame();
  const strike = interpolate(frame, [24, 54], [0, 100], clamp);
  return (
    <AbsoluteFill
      style={{
        display: "grid",
        placeItems: "center",
        color: colors.ink,
        textAlign: "center",
      }}
    >
      <div style={{ opacity: enter(frame, 0, 18), transform: `scale(${interpolate(enter(frame), [0, 1], [.96, 1])})` }}>
        <Kicker tone="red">The hard case</Kicker>
        <div
          style={{
            position: "relative",
            marginTop: 28,
            fontFamily: "Newsreader",
            fontSize: 96,
            lineHeight: 1,
            letterSpacing: "-0.05em",
          }}
        >
          The obvious theory
          <br />
          was wrong.
          <div
            style={{
              position: "absolute",
              left: "8%",
              right: `${100 - strike}%`,
              top: "26%",
              height: 6,
              borderRadius: 99,
              background: colors.red,
              transform: "rotate(-2deg)",
              boxShadow: "0 0 24px rgba(239,118,111,.45)",
            }}
          />
        </div>
        <p
          style={{
            margin: "30px auto 0",
            maxWidth: 720,
            color: colors.muted,
            fontFamily: "DM Sans",
            fontSize: 25,
            lineHeight: 1.4,
          }}
        >
          Clark kept the failed reproduction on screen, eliminated the timeout hypothesis, and changed direction.
        </p>
      </div>
    </AbsoluteFill>
  );
}

function FinalProof() {
  const frame = useCurrentFrame();
  const p = enter(frame, 0, 20);
  return (
    <AbsoluteFill style={{ padding: "58px 74px", color: colors.ink }}>
      <Window
        style={{
          position: "absolute",
          left: 74,
          width: 1180,
          top: 92,
          bottom: 82,
          opacity: p,
          transform: `translateX(${interpolate(p, [0, 1], [-54, 0])}px)`,
        }}
      >
        <ScreenImage src="final-proof.png" zoom={1.02} />
      </Window>
      <div
        style={{
          position: "absolute",
          right: 74,
          width: 500,
          top: 240,
          opacity: enter(frame, 12, 20),
        }}
      >
        <Kicker tone="green">The receipt</Kicker>
        <h2
          style={{
            margin: "24px 0 20px",
            fontFamily: "Newsreader",
            fontWeight: 430,
            fontSize: 74,
            lineHeight: .96,
            letterSpacing: "-0.05em",
          }}
        >
          Cause found.
          <br />
          <em style={{ color: colors.green }}>Failure gone.</em>
        </h2>
        <p style={{ color: colors.muted, fontFamily: "DM Sans", fontSize: 23, lineHeight: 1.4 }}>
          One scoped edit. Original fixture reproduced, then verified locally and on the affected runner.
        </p>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 10, marginTop: 28 }}>
          <ProofChip>14 passed</ProofChip>
          <ProofChip>0 failed</ProofChip>
          <ProofChip tone="neutral">1 file</ProofChip>
        </div>
      </div>
    </AbsoluteFill>
  );
}

function EndCard() {
  const frame = useCurrentFrame();
  const p = enter(frame, 0, 20);
  return (
    <AbsoluteFill
      style={{
        display: "grid",
        placeItems: "center",
        color: colors.ink,
        textAlign: "center",
      }}
    >
      <div style={{ opacity: p, transform: `translateY(${interpolate(p, [0, 1], [24, 0])}px)` }}>
        <div style={{ display: "flex", justifyContent: "center", marginBottom: 28 }}><Brand /></div>
        <h2
          style={{
            margin: 0,
            fontFamily: "Newsreader",
            fontSize: 86,
            fontWeight: 430,
            lineHeight: .98,
            letterSpacing: "-0.05em",
          }}
        >
          Agent work you can see—
          <br />
          <em style={{ color: colors.accent }}>and trust.</em>
        </h2>
        <div
          style={{
            display: "inline-flex",
            marginTop: 42,
            padding: "15px 24px",
            borderRadius: 999,
            background: colors.paper,
            color: colors.black,
            fontFamily: "JetBrains Mono",
            fontSize: 18,
            letterSpacing: ".04em",
          }}
        >
          clarkchat.com/clark-code
        </div>
      </div>
    </AbsoluteFill>
  );
}

export function FlagshipFilm() {
  return (
    <AbsoluteFill style={{ background: colors.black }}>
      <Fonts />
      <Atmosphere />
      <Sequence from={0} durationInFrames={90}><ProofOpen /></Sequence>
      <Sequence from={90} durationInFrames={120}><Problem /></Sequence>
      <Sequence from={210} durationInFrames={180}>
        <ScreenRun sourceStart={0} label="01 / Reproduce" headline="Start with the failure, not the guess." camera="bottom" />
      </Sequence>
      <Sequence from={390} durationInFrames={210}>
        <ScreenRun sourceStart={6} label="02 / Investigate" headline="Three bounded threads. One question." />
      </Sequence>
      <Sequence from={600} durationInFrames={90}><WrongTheory /></Sequence>
      <Sequence from={690} durationInFrames={210}>
        <ScreenRun sourceStart={12.3} label="03 / Find the boundary" headline="Root cause, with the evidence attached." camera="bottom" />
      </Sequence>
      <Sequence from={900} durationInFrames={210}>
        <ScreenRun sourceStart={19.1} label="04 / Verify" headline="One-line fix. Two machines." tone="green" camera="bottom" />
      </Sequence>
      <Sequence from={1110} durationInFrames={90}><FinalProof /></Sequence>
      <Sequence from={1200} durationInFrames={60}><EndCard /></Sequence>
    </AbsoluteFill>
  );
}
