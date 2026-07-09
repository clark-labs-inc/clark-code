/**
 * Design-system integration demo for clark-desktop — ADDITIVE, standalone entry
 * (ds-demo.html). Proves the SAME `@clark-labs/design-system/web` package that
 * clark-ui consumes also renders inside the desktop app's Vite + Tailwind v4 +
 * Tauri WebView environment, in Unified Graphite. Deliberately does not touch
 * App.tsx / main.tsx (concurrently edited).
 */
import React from 'react';
import ReactDOM from 'react-dom/client';
import '@clark-labs/design-system/theme.css';
import '@clark-labs/design-system/web/styles.css';
import {
  Button,
  Card,
  Chip,
  Stack,
  Text,
  BarChart,
  StatTile,
  MotionEnter,
  MotionLoop,
  useTheme,
  resolveTheme,
  categoricalHues,
  type ButtonVariant,
} from '@clark-labs/design-system/web';

// Desktop is dark-default (html.dark); seed the DS theme to match on first load.
if (!localStorage.getItem('clark.theme')) localStorage.setItem('clark.theme', 'dark');

const VARIANTS: ButtonVariant[] = ['primary', 'secondary', 'outline', 'ghost', 'danger'];
const BARS = [8, 14, 6, 18, 11, 15];

function Section({ label, title, children }: { label: string; title: string; children: React.ReactNode }) {
  return (
    <section style={{ padding: '24px 0', borderTop: '1px solid var(--color-border-subtle)' }}>
      <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-2xs)', letterSpacing: '0.1em', textTransform: 'uppercase', color: 'var(--color-accent)', marginBottom: 12 }}>
        {label}
      </div>
      <h2 style={{ fontFamily: 'var(--font-display)', fontSize: 'var(--text-2xl)', margin: '0 0 16px', fontWeight: 700 }}>{title}</h2>
      {children}
    </section>
  );
}

function Demo() {
  const { mode, toggle } = useTheme();
  const theme = resolveTheme(mode);
  return (
    <div style={{ minHeight: '100vh', background: 'var(--color-bg-primary)', color: 'var(--color-text-primary)', fontFamily: 'var(--font-sans)' }}>
      <div style={{ maxWidth: 880, margin: '0 auto', padding: '32px 24px 80px' }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 16 }}>
          <div>
            <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-2xs)', letterSpacing: '0.1em', textTransform: 'uppercase', color: 'var(--color-text-faint)' }}>
              clark-desktop · @clark-labs/design-system
            </div>
            <h1 style={{ fontFamily: 'var(--font-display)', fontSize: 'var(--text-4xl)', margin: '4px 0 0', fontWeight: 800 }}>Unified Graphite in the desktop shell</h1>
          </div>
          <Button variant="secondary" onPress={toggle}>{mode === 'dark' ? '☾ dark' : '☀ light'}</Button>
        </div>
        <p style={{ color: 'var(--color-text-muted)', maxWidth: '62ch', lineHeight: 1.6 }}>
          The same package clark-ui uses, running in the Tauri WebView. The theme toggle drives the
          shared token core; because the DS dark selector also targets <code>html.dark</code>, it
          reconciles with desktop's own dark-mode mechanism.
        </p>

        <Section label="Evolve · greenfield" title="Data-viz palette">
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
            {categoricalHues.map((hue, i) => (
              <div key={hue} style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 5 }}>
                <div style={{ width: 40, height: 40, borderRadius: 10, background: theme.categorical[i] }} />
                <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-3xs)', color: 'var(--color-text-faint)' }}>{hue}</span>
              </div>
            ))}
          </div>
        </Section>

        <Section label="Layer 2 · primitives" title="Buttons, chips, cards">
          <Stack gap="md">
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 12 }}>{VARIANTS.map((v) => <Button key={v} variant={v}>{v}</Button>)}</div>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
              <Chip tone="neutral">neutral</Chip><Chip tone="accent">accent</Chip><Chip tone="success">success</Chip><Chip tone="warning">warning</Chip><Chip tone="danger">danger</Chip>
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))', gap: 14 }}>
              <Card variant="elevated"><Stack gap="xxs"><Text role="bodyStrong">Elevated</Text><Text role="bodySm" color="textMuted">shadow + hairline</Text></Stack></Card>
              <Card variant="outline"><Stack gap="xxs"><Text role="bodyStrong">Outline</Text><Text role="bodySm" color="textMuted">border only</Text></Stack></Card>
            </div>
          </Stack>
        </Section>

        <Section label="Layer 4 · charts" title="Charts (shared SVG geometry)">
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 14, alignItems: 'stretch' }}>
            <StatTile label="Sessions" value="312" delta={{ value: '8%', tone: 'up' }} spark={[4, 6, 5, 8, 7, 11, 9, 14]} />
            <Card variant="outline" padding="lg">
              <Text role="label" color="textFaint">Weekly volume</Text>
              <div style={{ marginTop: 8 }}><BarChart values={BARS} palette={theme.categorical} width={240} height={110} /></div>
            </Card>
          </div>
        </Section>

        <Section label="Layer 3 · motion" title="Motion">
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(180px, 1fr))', gap: 14 }}>
            <MotionEnter preset="fadeInUp"><Card variant="outline"><Text role="bodyStrong">fadeInUp</Text></Card></MotionEnter>
            <MotionEnter preset="scaleIn"><Card variant="outline"><Text role="bodyStrong">scaleIn (spring)</Text></Card></MotionEnter>
            <MotionLoop preset="pulse"><Card variant="outline"><Text role="bodyStrong">pulse</Text></Card></MotionLoop>
          </div>
        </Section>
      </div>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Demo />
  </React.StrictMode>,
);
