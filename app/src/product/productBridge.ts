import { invoke } from "@tauri-apps/api/core";

const PRODUCT_OPERATION = /^[a-z0-9][a-z0-9._-]{0,127}$/;

export async function productRequest<T>(
  operation: string,
  payload: unknown = {},
): Promise<T> {
  if (!PRODUCT_OPERATION.test(operation)) {
    throw new Error("Product operation is invalid");
  }
  return invoke<T>("product_request", { operation, payload });
}
