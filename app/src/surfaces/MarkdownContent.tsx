import { useEffect, useState, type ReactNode } from "react";
import rehypeKatex from "rehype-katex";
import remarkMath from "remark-math";
import {
  Streamdown,
  type Components as StreamdownComponents,
  type StreamdownProps,
} from "streamdown";
import { Check, Copy } from "lucide-react";
import { useCopy } from "../lib/clipboard";
import { markdownUrlTransform } from "../lib/fileLinks";
import { highlight, resolveLang } from "../lib/highlight";
import { MarkdownLink } from "./MarkdownLink";
import { Mermaid } from "./work/Mermaid";

export const MARKDOWN_CLASSES =
  "text-ink [&_p]:my-2.5 [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 " +
  "[&_ul]:my-2.5 [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:marker:text-ink-faint [&_ol]:my-2.5 [&_ol]:list-decimal [&_ol]:pl-5 [&_ol]:marker:text-ink-faint [&_li]:my-1 " +
  "[&_h1]:mb-1.5 [&_h1]:mt-4 [&_h1]:text-lg [&_h1]:font-semibold [&_h1]:tracking-tight [&_h2]:mb-1.5 [&_h2]:mt-4 [&_h2]:font-semibold [&_h2]:tracking-tight [&_h3]:mb-1 [&_h3]:mt-3 [&_h3]:font-semibold " +
  "[&_a]:text-ink [&_a]:underline [&_a]:decoration-ink-faint [&_a]:underline-offset-2 hover:[&_a]:decoration-ink [&_strong]:font-semibold [&_strong]:text-ink " +
  "[&_pre]:my-2 [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:border [&_pre]:border-border-subtle [&_pre]:bg-bg-sunken [&_pre]:p-3 [&_pre]:font-mono [&_pre]:text-xs [&_pre]:leading-relaxed [&_pre>code]:bg-transparent [&_pre>code]:p-0 [&_pre>code]:border-0 " +
  "[&_:not(pre)>code]:rounded-[4px] [&_:not(pre)>code]:bg-chip [&_:not(pre)>code]:px-[0.3em] [&_:not(pre)>code]:py-[0.08em] [&_:not(pre)>code]:font-mono [&_:not(pre)>code]:text-[0.84em] [&_:not(pre)>code]:text-ink-secondary " +
  "[&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_blockquote]:text-ink-muted " +
  "[&_table]:my-2 [&_table]:w-full [&_table]:border-collapse [&_table]:table-fixed [&_table]:text-xs " +
  "[&_th]:border [&_th]:border-border-subtle [&_th]:px-2 [&_th]:py-1.5 [&_th]:text-left [&_th]:align-top [&_th]:font-medium [&_th]:text-ink-secondary [&_th]:break-words " +
  "[&_td]:border [&_td]:border-border-subtle [&_td]:px-2 [&_td]:py-1.5 [&_td]:align-top [&_td]:break-words [&_td]:overflow-wrap-anywhere";

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

function CodeBlock({ lang, code }: { lang?: string; code: string }) {
  const [copied, copy] = useCopy();
  const resolved = resolveLang(lang);
  const [html, setHtml] = useState<string | null>(null);

  useEffect(() => {
    setHtml(null);
    if (!resolved) return;
    let alive = true;
    void highlight(code, lang).then((result) => {
      if (alive && result.html) setHtml(result.html);
    });
    return () => {
      alive = false;
    };
  }, [code, lang, resolved]);

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

function components(diagrams: boolean): StreamdownComponents {
  // Keep native semantic tags explicit: Streamdown's defaults may substitute
  // styled primitives, which changes both accessibility and inherited CSS.
  return {
    p: "p",
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
    img: "img",
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
      <div className="overflow-x-auto">
        <table {...props} />
      </div>
    ),
  };
}

const STATIC_COMPONENTS = components(false);
const DIAGRAM_COMPONENTS = components(true);

export function MarkdownContent({
  children,
  className,
  math = false,
  diagrams = false,
  mode = "static",
  animated = false,
  isAnimating = false,
}: {
  children: string;
  className?: string;
  math?: boolean;
  diagrams?: boolean;
  mode?: "static" | "streaming";
  animated?: StreamdownProps["animated"];
  isAnimating?: boolean;
}) {
  return (
    <Streamdown
      mode={mode}
      className={className}
      animated={animated}
      components={diagrams ? DIAGRAM_COMPONENTS : STATIC_COMPONENTS}
      controls={false}
      isAnimating={isAnimating}
      parseIncompleteMarkdown={mode === "streaming"}
      rehypePlugins={math ? [rehypeKatex] : undefined}
      remarkPlugins={math ? [remarkMath] : undefined}
      skipHtml
      urlTransform={markdownUrlTransform}
    >
      {children}
    </Streamdown>
  );
}
