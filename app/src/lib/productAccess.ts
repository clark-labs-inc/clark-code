import { productRequest } from "../product/productBridge";

export type ProductAccessAvailability = "available" | "blocked" | "unknown";

export interface ProductAccessCapability {
  id: string;
  availability: ProductAccessAvailability;
  reason: string;
  title: string;
  detail: string;
  actionLabel?: string;
  actionUrl?: string;
}

export interface ProductAccessProjection {
  schemaVersion: 1;
  account: {
    kind: string;
    label: string;
  };
  capabilities: ProductAccessCapability[];
  usage: {
    state: string;
    label: string;
    percentUsed: number;
    isUnlimited: boolean;
  };
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} is invalid`);
  }
  return value as Record<string, unknown>;
}

function string(value: unknown, label: string): string {
  if (typeof value !== "string" || !value.trim()) throw new Error(`${label} is invalid`);
  return value;
}

function optionalString(value: unknown, label: string): string | undefined {
  return value === undefined || value === null ? undefined : string(value, label);
}

function optionalActionUrl(value: unknown, label: string): string | undefined {
  const candidate = optionalString(value, label);
  if (!candidate) return undefined;
  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    throw new Error(`${label} is invalid`);
  }
  if (url.protocol !== "https:" || url.username || url.password) {
    throw new Error(`${label} is invalid`);
  }
  return url.toString();
}

export function parseProductAccess(value: unknown): ProductAccessProjection {
  const root = record(value, "Product access");
  if (root.schema_version !== 1 || !Array.isArray(root.capabilities)) {
    throw new Error("Product access schema is unsupported");
  }
  const account = record(root.account, "Product access account");
  const usage = record(root.usage, "Product access usage");
  const percentUsed = usage.percent_used;
  if (
    typeof percentUsed !== "number"
    || !Number.isInteger(percentUsed)
    || percentUsed < 0
    || percentUsed > 100
    || typeof usage.is_unlimited !== "boolean"
  ) {
    throw new Error("Product access usage is invalid");
  }
  const capabilities = root.capabilities.map((entry, index): ProductAccessCapability => {
    const capability = record(entry, `Product capability ${index}`);
    const availability = capability.availability;
    if (availability !== "available" && availability !== "blocked" && availability !== "unknown") {
      throw new Error(`Product capability ${index} availability is invalid`);
    }
    return {
      id: string(capability.id, `Product capability ${index} id`),
      availability,
      reason: string(capability.reason, `Product capability ${index} reason`),
      title: string(capability.title, `Product capability ${index} title`),
      detail: string(capability.detail, `Product capability ${index} detail`),
      actionLabel: optionalString(
        capability.action_label,
        `Product capability ${index} action label`,
      ),
      actionUrl: optionalActionUrl(
        capability.action_url,
        `Product capability ${index} action URL`,
      ),
    };
  });
  if (new Set(capabilities.map(({ id }) => id)).size !== capabilities.length) {
    throw new Error("Product capability ids must be unique");
  }
  return {
    schemaVersion: 1,
    account: {
      kind: string(account.kind, "Product access account kind"),
      label: string(account.label, "Product access account label"),
    },
    capabilities,
    usage: {
      state: string(usage.state, "Product access usage state"),
      label: string(usage.label, "Product access usage label"),
      percentUsed,
      isUnlimited: usage.is_unlimited,
    },
  };
}

export async function productAccessSnapshot(): Promise<ProductAccessProjection> {
  return parseProductAccess(await productRequest<unknown>("access.snapshot"));
}

export function capabilityAccess(
  projection: ProductAccessProjection | null,
  id: string,
): ProductAccessCapability | null {
  return projection?.capabilities.find((capability) => capability.id === id) ?? null;
}
