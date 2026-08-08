export async function fetchPreview(destination: string) {
  // Vulnerable: a tenant chooses an unrestricted server-side destination.
  const response = await fetch(destination, { redirect: "follow" });
  return response.text();
}
