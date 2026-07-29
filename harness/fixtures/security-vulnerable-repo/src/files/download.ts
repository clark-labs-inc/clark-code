import { readFile } from "node:fs/promises";
import { join } from "node:path";

const DOWNLOAD_ROOT = "/srv/downloads";

export async function download(name: string) {
  // Vulnerable: join does not stop ../../ traversal outside DOWNLOAD_ROOT.
  return readFile(join(DOWNLOAD_ROOT, name));
}
