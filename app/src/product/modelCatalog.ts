// YAML-driven model catalog.
//
// The picker's model choices are the single source of truth in `models.yaml`,
// loaded here and validated into the typed `ProductModelOption` shape. A branded
// composition can supply its own catalog (for example one backed by Clark's
// hosted/DynamoDB model registry) at build time; the neutral build reads the
// checked-in YAML file.
import { parse } from "yaml";
import type { ProductModelOption } from "./productModule";
import modelsYaml from "./models.yaml?raw";

interface CatalogEntry {
  id: unknown;
  label: unknown;
  hint: unknown;
  defaultReasoningEffort: unknown;
}

interface CatalogFile {
  default?: unknown;
  defaultReasoningEffort?: unknown;
  models?: unknown;
}

const REASONING_EFFORTS = new Set([
  "",
  "max",
  "xhigh",
  "high",
  "medium",
  "low",
  "minimal",
]);

function isModelOption(value: unknown): value is ProductModelOption {
  const entry = value as CatalogEntry;
  return (
    typeof entry?.id === "string"
    && entry.id.length > 0
    && typeof entry?.label === "string"
    && entry.label.length > 0
    && typeof entry?.hint === "string"
    && typeof entry?.defaultReasoningEffort === "string"
    && REASONING_EFFORTS.has(entry.defaultReasoningEffort)
  );
}

export interface ModelCatalog {
  defaultModel: string;
  defaultReasoningEffort: ProductModelOption["defaultReasoningEffort"];
  models: readonly ProductModelOption[];
}

function parseCatalog(raw: string): ModelCatalog {
  let parsed: CatalogFile;
  try {
    parsed = parse(raw) as CatalogFile;
  } catch {
    parsed = {};
  }
  const entries = Array.isArray(parsed?.models)
    ? (parsed.models as unknown[]).filter(isModelOption)
    : [];
  const defaultModel = typeof parsed?.default === "string" && parsed.default.length > 0
    ? parsed.default
    : entries[0]?.id ?? "local-model";
  const hasDefault = entries.some((entry) => entry.id === defaultModel);
  if (entries.length === 0) {
    throw new Error("Model catalog YAML contains no valid model options.");
  }
  if (!hasDefault) {
    throw new Error(`Model catalog default "${defaultModel}" is not a listed model.`);
  }
  const defaultEffort = (
    typeof parsed?.defaultReasoningEffort === "string"
    && REASONING_EFFORTS.has(parsed.defaultReasoningEffort)
    && parsed.defaultReasoningEffort !== ""
  ) ? parsed.defaultReasoningEffort as ProductModelOption["defaultReasoningEffort"]
    : "high";
  return {
    defaultModel,
    defaultReasoningEffort: defaultEffort,
    models: entries,
  };
}

const catalog = parseCatalog(modelsYaml);

/** The neutral build's model catalog, loaded from `models.yaml`. */
export const modelCatalog: ModelCatalog = catalog;