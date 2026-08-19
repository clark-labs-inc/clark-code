import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { Check, ChevronRight, Loader2, Sparkles } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { cn } from "../../lib/cn";
import {
  clearComposerDraftIfUnchanged,
  composerDraftOwner,
  composerDraftRef,
  specialistStartComposerDraftId,
} from "../../lib/composerDraft";
import { accessibleMotion, RISE, RISE_SMALL, SLIDE_RIGHT, staggeredTransition } from "../../lib/motion";
import { guidedSpecPrompt, type SpecGuidanceReport } from "../../lib/specGuidance";
import { initialSpecDocument, preparedSpecDocumentPrompt } from "../../lib/specDocuments";
import { recordSpecPrompt } from "../../lib/specPromptHistory";
import { withActiveSpecialistSkill } from "../../lib/specialists";
import { productModule } from "../../product/productModule";
import { useSessionStore } from "../../store/sessionStore";

export interface SpecGuidedPreview {
  answer: string;
  phase: "draft" | "sent";
}

export function SpecGuidedDocumentCue({
  report,
  preview,
  busy,
}: {
  report: SpecGuidanceReport;
  preview: SpecGuidedPreview | null;
  busy: boolean;
}) {
  const reduceMotion = useReducedMotion();
  if (report.complete) return null;

  return (
    <m.section
      layout={!reduceMotion}
      data-qa="spec-guided-document-cue"
      className="my-7 overflow-hidden rounded-xl border border-accent/30 bg-accent-subtle/45 shadow-[0_14px_45px_-34px_var(--color-accent)]"
      {...accessibleMotion(RISE, reduceMotion)}
    >
      <div className="flex items-center gap-2 border-b border-accent/15 px-4 py-2.5 text-xs">
        <span className="grid size-5 place-items-center rounded-full bg-accent text-on-accent">
          <Sparkles aria-hidden="true" className="size-3" />
        </span>
        <span className="font-semibold text-accent">Shaping now</span>
        <span className="text-ink-faint">·</span>
        <span className="truncate text-ink-muted">{report.current.label}</span>
      </div>
      <div className="px-4 py-4">
        <p className="font-serif text-lg font-semibold leading-6 text-ink">
          {report.current.question}
        </p>
        <AnimatePresence initial={false} mode="popLayout">
          {preview ? (
            <m.div
              key={`${report.current.id}:${preview.answer}`}
              {...accessibleMotion(RISE_SMALL, reduceMotion)}
              className="mt-3"
            >
              <p className="text-sm leading-6 text-accent">“{preview.answer}”</p>
              <div className="mt-3 flex flex-wrap items-center gap-2 text-xs">
                <span className="text-ink-faint">Your words will clarify</span>
                <span className="rounded-full border border-success/25 bg-success/10 px-2 py-1 font-medium text-success">
                  {report.current.label}
                </span>
                <span className="rounded-full border border-success/25 bg-success/10 px-2 py-1 font-medium text-success">
                  Acceptance
                </span>
                {(busy || preview.phase === "sent") && (
                  <span className="flex items-center gap-1.5 text-accent">
                    <Loader2 aria-hidden="true" className="size-3 animate-[spin_1s_linear_infinite]" />
                    Sculpting into the document…
                  </span>
                )}
              </div>
            </m.div>
          ) : (
            <m.p
              key="empty"
              {...accessibleMotion(RISE_SMALL, reduceMotion)}
              className="mt-2 text-xs leading-5 text-ink-faint"
            >
              Choose an answer beside the document. You will see the decision take shape here before it is folded into the spec.
            </m.p>
          )}
        </AnimatePresence>
      </div>
    </m.section>
  );
}

export function SpecGuidedInterview({
  report,
  busy,
  onPreview,
}: {
  report: SpecGuidanceReport;
  busy: boolean;
  onPreview: (preview: SpecGuidedPreview | null) => void;
}) {
  const session = useSessionStore((state) => state.session);
  const startSession = useSessionStore((state) => state.startSession);
  const send = useSessionStore((state) => state.send);
  const bridge = useSessionStore((state) => state.bridge);
  const cwd = useSessionStore((state) => state.activeProjectRoot ?? state.localSettings.cwd);
  const activeRemote = useSessionStore((state) => state.activeRemote);
  const auth = useSessionStore((state) => state.auth);
  const flashNotice = useSessionStore((state) => state.flashNotice);
  const setComposerPrefill = useSessionStore((state) => state.setComposerPrefill);
  const reduceMotion = useReducedMotion();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [custom, setCustom] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const answer = useMemo(() => custom.trim() || selected?.trim() || "", [custom, selected]);

  useEffect(() => {
    setSelected(null);
    setCustom("");
    setSubmitting(false);
    onPreview(null);
  }, [onPreview, report.current.id]);

  useEffect(() => {
    if (!answer) {
      onPreview(null);
      return;
    }
    onPreview({ answer, phase: submitting ? "sent" : "draft" });
  }, [answer, onPreview, submitting]);

  const choose = (option: string) => {
    setSelected(option);
    setCustom("");
  };

  const queueInComposer = (text: string) => {
    setComposerPrefill(text);
    flashNotice("Your answer is ready below. Send it to start shaping the spec.");
    requestAnimationFrame(() => {
      const composer = document.querySelector<HTMLTextAreaElement>("textarea.composer-input");
      composer?.focus();
      composer?.setSelectionRange(text.length, text.length);
    });
  };

  const submit = async () => {
    if (!answer || busy || submitting) return;
    const plainPrompt = `For “${report.current.question}”, my answer is: ${answer}`;
    setSubmitting(true);
    let targetSession = session;
    let preservedComposerDraft = "";
    if (!targetSession) {
      const owner = composerDraftOwner(auth?.user ?? null);
      const existingComposerDraft = composerDraftRef.current.trim();
      const legacyGuidedPrefill = existingComposerDraft === plainPrompt;
      preservedComposerDraft = legacyGuidedPrefill ? "" : existingComposerDraft;
      if (legacyGuidedPrefill) {
        clearComposerDraftIfUnchanged(
          owner,
          specialistStartComposerDraftId("spec"),
          existingComposerDraft,
        );
      }
      await startSession({ submittedDraft: plainPrompt });
      const startedState = useSessionStore.getState();
      targetSession = startedState.session;
      if (!targetSession) {
        flashNotice(startedState.error ?? "Could not start this specification. Try again.");
        setSubmitting(false);
        return;
      }
      // Starting the conversation remounts the persistent composer. Restore
      // only text the person authored there; never expose the guided workflow
      // envelope as a second manual send step.
      startedState.setComposerPrefill(preservedComposerDraft);
    }
    const liveState = useSessionStore.getState();
    const skillCwd = liveState.activeProjectRoot ?? liveState.localSettings.cwd ?? cwd;
    const liveRemote = liveState.activeRemote ?? activeRemote;
    const remote = liveRemote ? { id: liveRemote.id } : null;
    let catalog = await bridge?.listSkills?.(skillCwd, remote);
    let references = withActiveSpecialistSkill([], catalog?.skills ?? [], "spec", "spec:spec");
    if (references.length === 0 && bridge?.reloadSkills) {
      try {
        catalog = await bridge.reloadSkills(skillCwd, remote);
        references = withActiveSpecialistSkill([], catalog.skills, "spec", "spec:spec");
      } catch {
        // The specific unavailable-workflow message below is more actionable.
      }
    }
    if (references.length === 0) {
      flashNotice("The Spec workflow is unavailable. Reload skills and try again.");
      setSubmitting(false);
      return;
    }
    let prepared: { filename: string } | null | undefined;
    try {
      prepared = await productModule().specialistWorkspace?.prepareDocument?.(
        "spec",
        targetSession.id,
        initialSpecDocument(plainPrompt),
      );
    } catch {
      flashNotice("Could not load the saved spec. Try again.");
      setSubmitting(false);
      return;
    }
    const guidedPrompt = guidedSpecPrompt(report.current, answer);
    const outcome = await send(
      prepared ? preparedSpecDocumentPrompt(guidedPrompt, prepared.filename) : guidedPrompt,
      references,
    );
    if (outcome.kind === "not_sent") {
      setSubmitting(false);
      return;
    }
    recordSpecPrompt(composerDraftOwner(auth?.user ?? null), targetSession.id, plainPrompt);
    setSubmitting(false);
  };

  if (report.complete) {
    return (
      <m.aside
        data-qa="spec-guided-interview"
        className="flex h-[56%] min-h-[20rem] max-h-[32rem] w-full shrink-0 flex-col border-t border-border-subtle bg-bg-secondary/55 p-5 lg:h-auto lg:min-h-0 lg:max-h-none lg:w-[22rem] lg:border-l lg:border-t-0 xl:w-[25rem]"
        {...accessibleMotion(SLIDE_RIGHT, reduceMotion)}
      >
        <div className="rounded-xl border border-success/25 bg-success/10 p-4">
          <div className="flex items-center gap-2 text-sm font-semibold text-success">
            <Check className="size-4" /> Ready for agent planning
          </div>
          <p className="mt-2 text-xs leading-5 text-ink-muted">
            All {report.total} decision areas have substantive coverage. An engineering agent can now plan from the document without relying on this conversation.
          </p>
          <button
            type="button"
            onClick={() => queueInComposer("Stress-test this specification for contradictions, missing recovery behavior, and acceptance criteria that are not observable. Update the living SPEC.md with the findings.")}
            className="mt-4 flex h-9 w-full items-center justify-center gap-2 rounded-lg border border-success/30 bg-bg px-3 text-xs font-semibold text-ink hover:bg-bg-hover"
          >
            Stress-test the spec <ChevronRight className="size-3.5" />
          </button>
        </div>
      </m.aside>
    );
  }

  return (
    <m.aside
      data-qa="spec-guided-interview"
      className="flex h-[56%] min-h-[20rem] max-h-[32rem] w-full shrink-0 flex-col border-t border-border-subtle bg-bg-secondary/55 lg:h-auto lg:min-h-0 lg:max-h-none lg:w-[22rem] lg:border-l lg:border-t-0 xl:w-[25rem]"
      {...accessibleMotion(SLIDE_RIGHT, reduceMotion)}
    >
      <div className="border-b border-border-subtle px-5 pb-4 pt-5">
        <div className="flex items-center justify-between gap-3 text-xs">
          <span className="font-semibold text-ink">Guided interview</span>
          <span className="text-ink-faint">{report.clear} of {report.total} clear</span>
        </div>
        <div className="mt-3 grid grid-cols-8 gap-1" aria-label={`${report.clear} of ${report.total} decisions clear`}>
          {Array.from({ length: report.total }, (_, index) => (
            <span
              key={index}
              className={cn("h-1 rounded-full", index < report.clear ? "bg-success" : index === report.clear ? "bg-accent" : "bg-border")}
            />
          ))}
        </div>
        <p className="mt-3 text-xs leading-5 text-ink-faint">One approachable question at a time. Your answer reshapes the living document.</p>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5">
        <m.div key={report.current.id} {...accessibleMotion(RISE, reduceMotion)}>
          <p className="text-xs font-semibold uppercase tracking-[0.12em] text-accent">Your input</p>
          <h2 className="mt-2 font-serif text-2xl font-semibold leading-8 tracking-[-0.025em] text-ink">
            {report.current.question}
          </h2>
          <div className="mt-4 space-y-2 sm:grid sm:grid-cols-2 sm:gap-2 sm:space-y-0 lg:block lg:space-y-2">
            {report.current.options.map((option, index) => (
              <m.button
                key={option}
                type="button"
                onClick={() => choose(option)}
                aria-pressed={selected === option && !custom}
                className={cn(
                  "flex min-h-11 w-full items-center gap-3 rounded-lg border px-3 py-2.5 text-left text-sm leading-5 transition-colors",
                  selected === option && !custom
                    ? "border-accent/55 bg-accent-subtle text-ink"
                    : "border-border-subtle bg-bg text-ink-secondary hover:border-accent/30 hover:bg-bg-hover hover:text-ink",
                )}
                {...accessibleMotion(RISE_SMALL, reduceMotion)}
                transition={staggeredTransition(reduceMotion, index, 0.04)}
              >
                <span className={cn(
                  "grid size-5 shrink-0 place-items-center rounded-full border",
                  selected === option && !custom ? "border-accent bg-accent text-on-accent" : "border-border text-transparent",
                )}>
                  <Check className="size-3" />
                </span>
                <span>{option}</span>
              </m.button>
            ))}
          </div>

          <label className="mt-5 block text-xs font-medium text-ink-muted" htmlFor="spec-guided-answer">Say it your way</label>
          <textarea
            ref={inputRef}
            id="spec-guided-answer"
            value={custom}
            onChange={(event) => {
              setCustom(event.target.value);
              if (event.target.value) setSelected(null);
            }}
            rows={3}
            placeholder={report.current.placeholder}
            className="mt-2 w-full resize-none rounded-lg border border-border bg-bg px-3 py-2.5 text-sm leading-5 text-ink outline-none transition focus:border-accent/60 placeholder:text-ink-faint"
          />
        </m.div>
      </div>

      <div className="border-t border-border-subtle px-5 py-4">
        <div className="mb-4 hidden rounded-lg bg-bg px-3 py-2.5 lg:block">
          <p className="text-xs font-semibold text-ink">Why this matters</p>
          <p className="mt-1 text-xs leading-5 text-ink-faint">{report.current.why}</p>
        </div>
        <button
          type="button"
          onClick={() => void submit()}
          disabled={!answer || busy || submitting}
          className="flex h-10 w-full items-center justify-center gap-2 rounded-lg bg-accent px-4 text-sm font-semibold text-on-accent transition hover:bg-accent-hover disabled:bg-bg-hover disabled:text-ink-faint"
        >
          {submitting || busy ? (
            <><Loader2 className="size-4 animate-[spin_1s_linear_infinite]" /> Shaping the spec…</>
          ) : (
            <>Next question <ChevronRight className="size-4" /></>
          )}
        </button>
      </div>
    </m.aside>
  );
}
