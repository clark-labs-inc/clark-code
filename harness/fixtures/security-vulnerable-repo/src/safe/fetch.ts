const ALLOWED_HOSTS = new Set(["images.example.test"]);

export async function fetchAvatar(raw: string) {
  const target = new URL(raw);
  if (target.protocol !== "https:" || !ALLOWED_HOSTS.has(target.hostname)) {
    throw new Error("destination is not allowed");
  }
  return fetch(target, { redirect: "error" });
}
