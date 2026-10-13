import { useEffect, useState } from "react";
import type { Artifact } from "../core-bridge/types";
import { useSessionStore } from "../store/sessionStore";
import { artifactAvailability, preferredArtifact } from "./artifactPresentation";

export function useArtifactPanelSelection({
  sessionId,
  artifactCount,
  subagentsOpen,
  closeSubagents,
}: {
  sessionId: string | undefined;
  artifactCount: number;
  subagentsOpen: boolean;
  closeSubagents: () => void;
}) {
  const [artifactPanelOpen, setArtifactPanelOpen] = useState(false);
  const [activeArtifactId, setActiveArtifactId] = useState<string | null>(null);
  const [selectedArtifact, setSelectedArtifact] = useState<Artifact | null>(null);

  const closeArtifactPanel = () => {
    setArtifactPanelOpen(false);
    setActiveArtifactId(null);
    setSelectedArtifact(null);
  };

  useEffect(() => {
    closeArtifactPanel();
  }, [sessionId]);

  useEffect(() => {
    const artifacts = useSessionStore.getState().snapshot.artifacts;
    if (activeArtifactId
      && selectedArtifact?.id !== activeArtifactId
      && !artifacts.some((artifact) => artifact.id === activeArtifactId)) {
      setActiveArtifactId(null);
    }
  }, [activeArtifactId, artifactCount, selectedArtifact]);

  useEffect(() => {
    if (subagentsOpen && artifactPanelOpen) closeArtifactPanel();
  }, [artifactPanelOpen, subagentsOpen]);

  const openArtifact = (artifact: Artifact) => {
    closeSubagents();
    setArtifactPanelOpen(true);
    setActiveArtifactId(artifact.id);
    setSelectedArtifact(artifact);
  };

  const openArtifacts = () => {
    const state = useSessionStore.getState();
    if (!state.session) return;
    const artifacts = state.snapshot.artifacts;
    const latest = preferredArtifact(artifacts);
    closeSubagents();
    setArtifactPanelOpen(true);
    setSelectedArtifact(null);
    setActiveArtifactId((current) =>
      current && artifacts.some((artifact) => artifact.id === current && artifactAvailability(artifact) !== "unavailable")
        ? current
        : (latest?.id ?? null),
    );
  };

  const jumpToSource = (artifact: Artifact) => {
    const targetId = artifact.tool_call ? `tool-call-${artifact.tool_call}` : `artifact-${artifact.id}`;
    const target = document.getElementById(targetId);
    if (!target) {
      useSessionStore.getState().flashNotice("The source message is in earlier history. Load earlier messages in the conversation to find it.");
      return;
    }
    closeArtifactPanel();
    requestAnimationFrame(() => {
      const reduceMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
      target.scrollIntoView({ behavior: reduceMotion ? "auto" : "smooth", block: "center" });
      target.focus({ preventScroll: true });
    });
  };

  return {
    artifactPanelOpen,
    activeArtifactId,
    selectedArtifact,
    setActiveArtifactId,
    closeArtifactPanel,
    openArtifact,
    openArtifacts,
    jumpToSource,
  };
}
