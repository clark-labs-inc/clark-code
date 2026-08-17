import type { ComponentType } from "react";
import type { ProductAccessProjection } from "../lib/productAccess";

export interface ProductBranding {
  id: string;
  name: string;
  shortName: string;
}

export interface ProductMarkProps {
  size?: number;
  tile?: boolean;
  className?: string;
}

export interface ProductSpecialistIconProps {
  className?: string;
}

export type ProductExceptionalState = "loading" | "empty" | "recovery";

export interface ProductExceptionalStateIllustrationProps {
  state: ProductExceptionalState;
  size?: number;
  className?: string;
  label?: string;
}

export interface ProductUiContext {
  access: ProductAccessProjection | null;
  accessLoading: boolean;
  accessError: string | null;
  reloadAccess: () => Promise<void>;
}

export interface ProductUiSlots {
  workspaceBefore?: ComponentType<ProductUiContext>;
  workspaceAfter?: ComponentType<ProductUiContext>;
  settings?: ComponentType<ProductUiContext>;
  account?: ComponentType<ProductUiContext>;
}

export interface ProductModelOption {
  id: string;
  label: string;
  hint: string;
  defaultReasoningEffort: "" | "max" | "xhigh" | "high" | "medium" | "low" | "minimal";
}

export interface ProductLocalAgentPolicy {
  defaultModel: string;
  defaultReasoningEffort: ProductModelOption["defaultReasoningEffort"];
  models: readonly ProductModelOption[];
  includedModel?: string;
  specialistModel?: ProductModelOption;
  providerExtra?: (context: ProductLocalAgentExtensionContext) => Record<string, unknown>;
  remoteSessionExtra?: (context: ProductLocalAgentExtensionContext) => Record<string, unknown>;
  gatedWorkflows?: readonly ProductGatedWorkflow[];
  workflowAccess?: ProductWorkflowAccessCopy;
}

export interface ProductGatedWorkflow {
  command: string;
  skill?: string;
  label: string;
  hint: string;
  value: string;
}

export interface ProductWorkflowAccessCopy {
  capability: string;
  badge: string;
  available: string;
  checking: string;
  unavailable: string;
  actionLabel: string;
}

export interface ProductLocalAgentExtensionContext {
  specialist?: {
    organizationId: string;
    kind: string;
    workflow: string;
  };
  trainingOptIn: boolean;
}

export interface ProductVoiceInput {
  filename: string;
  contentType: string;
  dataBase64: string;
}

export interface ProductVoiceTranscription {
  text: string;
  model?: string;
  format?: string;
}

export interface ProductVoiceStreamSession {
  id: string;
}

export interface ProductVoiceStreamPolicy {
  start: () => Promise<ProductVoiceStreamSession>;
  send: (id: string, dataBase64: string) => Promise<ProductVoiceTranscription>;
  finish: (id: string) => Promise<ProductVoiceTranscription>;
  cancel: (id: string) => Promise<void>;
}

export interface ProductVoicePolicy {
  transcribe?: (input: ProductVoiceInput) => Promise<ProductVoiceTranscription>;
  stream?: ProductVoiceStreamPolicy;
}

export interface ProductSpecialistWorkspacePolicy {
  isConversationBound: (kind: string) => boolean;
  prepareDocument?: (kind: string, conversationId: string) => Promise<void>;
}

export interface ProductArtifactPolicy {
  isCloudUri: (uri: string) => boolean;
  cloudLabel: string;
  cloudDescription: string;
}

export interface ProductErrorPolicy {
  isAccountReconnectError: (raw: string) => boolean;
}

export interface ProductUsageFailureProps {
  error?: string | null;
  includedModel: boolean;
}

export interface ProductSpecialistAccessCopy {
  title: string;
  detail: string;
  action: "sign_in" | "product_action" | "retry" | "setup_workspace" | null;
  actionLabel?: string;
}

export interface ProductSpecialistAccessInput {
  state: string;
  kind: string;
  label: string;
  value: string;
}

export interface ProductModule {
  branding: ProductBranding;
  authRequired: boolean;
  mark?: ComponentType<ProductMarkProps>;
  exceptionalStateIllustration?: ComponentType<ProductExceptionalStateIllustrationProps>;
  slots: ProductUiSlots;
  localAgent: ProductLocalAgentPolicy;
  voice?: ProductVoicePolicy;
  specialistWorkspace?: ProductSpecialistWorkspacePolicy;
  artifacts: ProductArtifactPolicy;
  errors: ProductErrorPolicy;
  usageFailure?: ComponentType<ProductUsageFailureProps>;
  specialistAccessCopy?: (
    input: ProductSpecialistAccessInput,
  ) => ProductSpecialistAccessCopy;
  specialistCatalog?: unknown;
  specialistIcons?: Readonly<Record<string, ComponentType<ProductSpecialistIconProps>>>;
}

export const neutralProduct: ProductModule = Object.freeze({
  branding: {
    id: "clark_code",
    name: "Clark Code",
    shortName: "Clark Code",
  },
  authRequired: false,
  slots: {},
  localAgent: {
    defaultModel: "local-model",
    defaultReasoningEffort: "high" as const,
    models: [{
      id: "local-model",
      label: "Local model",
      hint: "OpenAI-compatible local coding model",
      defaultReasoningEffort: "high" as const,
    }, {
      id: "local-model-large",
      label: "Large local model",
      hint: "Higher-capacity OpenAI-compatible local model",
      defaultReasoningEffort: "max" as const,
    }],
  },
  artifacts: {
    isCloudUri: () => false,
    cloudLabel: "Product cloud",
    cloudDescription: "Saved securely in product cloud",
  },
  errors: {
    isAccountReconnectError: () => false,
  },
});

let activeProduct: ProductModule = neutralProduct;

export function installProductModule(product: ProductModule): void {
  const id = product.branding.id;
  if (!/^[a-z][a-z0-9_-]{1,63}$/.test(id)) {
    throw new Error("Product id is invalid");
  }
  activeProduct = Object.freeze({
    ...product,
    branding: Object.freeze({ ...product.branding }),
    slots: Object.freeze({ ...product.slots }),
    localAgent: Object.freeze({
      ...product.localAgent,
      models: Object.freeze(product.localAgent.models.map((model) => Object.freeze({ ...model }))),
    }),
    voice: product.voice
      ? Object.freeze({
          ...product.voice,
          stream: product.voice.stream ? Object.freeze({ ...product.voice.stream }) : undefined,
        })
      : undefined,
    specialistWorkspace: product.specialistWorkspace
      ? Object.freeze({ ...product.specialistWorkspace })
      : undefined,
    artifacts: Object.freeze({ ...product.artifacts }),
    errors: Object.freeze({ ...product.errors }),
    specialistIcons: product.specialistIcons
      ? Object.freeze({ ...product.specialistIcons })
      : undefined,
  });
}

export function productModule(): ProductModule {
  return activeProduct;
}

export function productName(): string {
  return activeProduct.branding.name;
}
