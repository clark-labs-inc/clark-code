import {
  Children,
  Fragment,
  createContext,
  isValidElement,
  useContext,
  useEffect,
  useRef,
  useState,
  type HTMLAttributes,
  type ReactNode,
} from "react";
import rehypeKatex from "rehype-katex";
import remarkMath from "remark-math";
import {
  Streamdown,
  defaultRehypePlugins,
  defaultRemarkPlugins,
  type Components as StreamdownComponents,
  type StreamdownProps,
} from "streamdown";
import { Check, Copy } from "lucide-react";
import { useCopy } from "../lib/clipboard";
import { markdownUrlTransform } from "../lib/fileLinks";
import { highlight, highlightCacheKey, resolveLang } from "../lib/highlight";
import { MarkdownLink } from "./MarkdownLink";
import { MarkdownImage } from "./MarkdownImage";
import { Mermaid } from "./work/Mermaid";

export const MARKDOWN_CLASSES =
  "text-ink-secondary [&_p]:my-3 [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 " +
  "[&_ul]:my-2.5 [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:marker:text-ink-faint [&_ol]:my-2.5 [&_ol]:list-decimal [&_ol]:pl-5 [&_ol]:marker:text-ink-faint [&_li]:my-1 " +
  "[&_h1]:mb-1.5 [&_h1]:mt-5 [&_h1]:text-lg [&_h1]:font-semibold [&_h1]:tracking-tight [&_h1]:text-ink [&_h2]:mb-1.5 [&_h2]:mt-5 [&_h2]:font-semibold [&_h2]:tracking-tight [&_h2]:text-ink [&_h3]:mb-1 [&_h3]:mt-4 [&_h3]:font-semibold [&_h3]:text-ink " +
  "[&_a]:text-ink [&_a]:underline [&_a]:decoration-ink-faint [&_a]:underline-offset-2 hover:[&_a]:decoration-ink [&_strong]:font-semibold [&_strong]:text-ink " +
  "[&_pre]:my-2 [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:border [&_pre]:border-border-subtle [&_pre]:bg-bg-sunken [&_pre]:p-3 [&_pre]:font-mono [&_pre]:text-xs [&_pre]:leading-relaxed [&_pre>code]:bg-transparent [&_pre>code]:p-0 [&_pre>code]:border-0 " +
  "[&_:not(pre)>code]:rounded-[4px] [&_:not(pre)>code]:bg-chip [&_:not(pre)>code]:px-[0.3em] [&_:not(pre)>code]:py-[0.08em] [&_:not(pre)>code]:font-mono [&_:not(pre)>code]:text-[0.84em] [&_:not(pre)>code]:text-ink-secondary " +
  "[&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_blockquote]:text-ink-muted " +
  "[&_table]:w-full [&_table]:table-fixed [&_th]:break-words [&_td]:break-words [&_td]:overflow-wrap-anywhere";

function codeFromPreChild(child: ReactNode): { lang?: string; code: string } | null {
  const element = Array.isArray(child)
    ? child.find((candidate) =>
      typeof candidate === "object" && candidate !== null && "props" in candidate
    )
    : child;
  if (typeof element !== "object" || element === null || !("props" in element)) return null;
  const props = (element as { props: { className?: string; children?: ReactNode } }).props;
  const lang = /language-(\S+)/.exec(props.className ?? "")?.[1];
  const inner = props.children;
  const code = typeof inner === "string" ? inner : Array.isArray(inner) ? inner.join("") : "";
  return { lang, code };
}

/** Whether the enclosing markdown is still arriving.
 *
 *  Streamdown owns the component tree between `MarkdownContent` and a code
 *  fence, so there is no prop path to thread this down; a context reaches the
 *  fence without changing the intermediate components' identities. */
const StreamingMarkdownContext = createContext(false);

/** Quiet period before tokenizing a fence that is still growing.
 *
 *  Tokenizing costs far more than a frame (~30 ms for 60 lines of TypeScript,
 *  ~130 ms for 300), so doing it per token cannot keep up: the main thread
 *  never finishes one pass before the next arrives and the UI advances in
 *  lurches. Waiting for the fence to stop changing turns O(tokens) passes into
 *  one, at the cost of showing plain monospace for this long after it settles. */
const STREAMING_HIGHLIGHT_QUIET_MS = 150;

function CodeBlock({ lang, code }: { lang?: string; code: string }) {
  const [copied, copy] = useCopy();
  const resolved = resolveLang(lang);
  const [html, setHtml] = useState<string | null>(null);
  const streaming = useContext(StreamingMarkdownContext);
  // What the current `html` was rendered from, so a slow in-flight highlight
  // cannot overwrite a newer one (the `alive` flag alone only covers unmount),
  // and so the effect can tell a fence that GREW from a fence that was
  // REPLACED. React reuses this instance for whatever fence lands at the same
  // tree position, so both the language and the source can change under us.
  const renderedFor = useRef<{ key: string; lang: string | undefined; code: string } | null>(null);

  useEffect(() => {
    const previous = renderedFor.current;
    if (!resolved) {
      // The new content has no highlightable language. Whatever `html` holds
      // belongs to a previous fence — showing it would put the old fence's
      // colored markup over this fence's text.
      if (previous !== null) {
        renderedFor.current = null;
        setHtml(null);
      }
      return;
    }
    const key = highlightCacheKey(lang, code);
    if (previous?.key === key) return;
    // Keep the previous highlight only while this is the same fence growing
    // (streamed source only ever gains a suffix). Same-language prefix growth
    // is that case; anything else — language change, rewritten content, an
    // instance reused for a different fence — must drop to plain text now
    // rather than show another fence's markup. The anti-strobe rule is scoped
    // to growth, where old and new content genuinely share a prefix.
    if (previous !== null && !(previous.lang === lang && code.startsWith(previous.code))) {
      renderedFor.current = null;
      setHtml(null);
    }
    let alive = true;
    const run = () => {
      void highlight(code, lang).then((result) => {
        if (!alive || !result.html) return;
        renderedFor.current = { key, lang, code };
        setHtml(result.html);
      });
    };
    // A settled fence highlights immediately; a growing one waits for quiet.
    if (!streaming) {
      run();
      return () => {
        alive = false;
      };
    }
    const timer = window.setTimeout(run, STREAMING_HIGHLIGHT_QUIET_MS);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [code, lang, resolved, streaming]);

  return (
    <div className="group/code relative">
      {html ? (
        <div className="shiki-host" dangerouslySetInnerHTML={{ __html: html }} />
      ) : (
        <pre>{code}</pre>
      )}
      <button
        type="button"
        onClick={() => copy(code)}
        aria-label={copied ? "Copied" : "Copy code"}
        title={copied ? "Copied" : "Copy code"}
        className="absolute right-2 top-2 grid size-7 place-items-center rounded-md bg-bg-elevated text-ink-faint opacity-0 ring-1 ring-border-subtle transition hover:text-ink group-hover/code:opacity-100"
      >
        {copied ? <Check className="size-3.5 text-success" /> : <Copy className="size-3.5" />}
      </button>
    </div>
  );
}

function MarkdownParagraph({ children, ...props }: HTMLAttributes<HTMLParagraphElement>) {
  const content = Children.toArray(children).filter((child) => child !== "");
  if (content.length === 1 && isValidElement(content[0]) && content[0].type === MarkdownImage) {
    return <Fragment>{children}</Fragment>;
  }
  return <p {...props}>{children}</p>;
}

function components(diagrams: boolean): StreamdownComponents {
  // Keep native semantic tags explicit: Streamdown's defaults may substitute
  // styled primitives, which changes both accessibility and inherited CSS.
  return {
    p: ({ node: _node, ...props }) => <MarkdownParagraph {...props} />,
    section: "section",
    ol: "ol",
    ul: "ul",
    li: "li",
    hr: "hr",
    strong: "strong",
    h1: "h1",
    h2: "h2",
    h3: "h3",
    h4: "h4",
    h5: "h5",
    h6: "h6",
    thead: "thead",
    tbody: "tbody",
    tr: "tr",
    th: "th",
    td: "td",
    blockquote: "blockquote",
    code: "code",
    img: ({ node: _node, ...props }) => <MarkdownImage {...props} />,
    sup: "sup",
    sub: "sub",
    a: ({ node: _node, ...props }) => <MarkdownLink {...props} />,
    pre: ({ node: _node, children }) => {
      const parsed = codeFromPreChild(children);
      if (!parsed) return <pre>{children}</pre>;
      if (diagrams && parsed.lang && /mermaid/i.test(parsed.lang)) {
        return <Mermaid code={parsed.code} />;
      }
      return <CodeBlock lang={parsed.lang} code={parsed.code} />;
    },
    table: ({ node: _node, ...props }) => (
      <div
        aria-label="Scrollable table"
        className="markdown-table-shell"
        data-markdown-table="true"
        role="region"
        tabIndex={0}
      >
        <table className="markdown-data-table" {...props} />
      </div>
    ),
  };
}

const STATIC_COMPONENTS = components(false);
const DIAGRAM_COMPONENTS = components(true);
// Streamdown's generic URL hardener replaces relative filesystem images before
// our component can resolve them. Keep raw parsing + sanitization; our stricter
// urlTransform and MarkdownImage own destination policy for this desktop app.
const MARKDOWN_REHYPE_PLUGINS = Object.entries(defaultRehypePlugins)
  .filter(([name]) => name !== "harden")
  .map(([, plugin]) => plugin);
const MARKDOWN_REMARK_PLUGINS = Object.values(defaultRemarkPlugins);
// Streamdown's per-block memo compares plugin-array identity, so building these
// inline would give every render a new array and re-parse the entire document
// each frame — the chat path avoids that only because it passes the constants
// above. Math surfaces need the same stability.
const MARKDOWN_REHYPE_PLUGINS_MATH = [...MARKDOWN_REHYPE_PLUGINS, rehypeKatex];
const MARKDOWN_REMARK_PLUGINS_MATH = [...MARKDOWN_REMARK_PLUGINS, remarkMath];

export function MarkdownContent({
  children,
  className,
  math = false,
  diagrams = false,
  mode = "static",
  repairIncomplete = false,
  animated = false,
  isAnimating = false,
}: {
  children: string;
  className?: string;
  math?: boolean;
  diagrams?: boolean;
  mode?: "static" | "streaming";
  /** Best-effort rendering for truncated or otherwise unfinished Markdown. */
  repairIncomplete?: boolean;
  animated?: StreamdownProps["animated"];
  isAnimating?: boolean;
}) {
  // `isAnimating` stays true for a beat after the last token while the entry
  // animation finishes; a fence is "still arriving" only while the source can
  // actually still change, which is what `mode` tracks.
  const streaming = mode === "streaming" || repairIncomplete;
  return (
    <StreamingMarkdownContext.Provider value={streaming}>
      <Streamdown
        mode={repairIncomplete ? "streaming" : mode}
        className={className}
        animated={animated}
        components={diagrams ? DIAGRAM_COMPONENTS : STATIC_COMPONENTS}
        controls={false}
        isAnimating={isAnimating}
        parseIncompleteMarkdown={streaming}
        rehypePlugins={math ? MARKDOWN_REHYPE_PLUGINS_MATH : MARKDOWN_REHYPE_PLUGINS}
        remarkPlugins={math ? MARKDOWN_REMARK_PLUGINS_MATH : MARKDOWN_REMARK_PLUGINS}
        skipHtml
        urlTransform={markdownUrlTransform}
      >
        {children}
      </Streamdown>
    </StreamingMarkdownContext.Provider>
  );
}
