import { createHash } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  readFileSync,
} from "node:fs";
import path from "node:path";

import {
  MACOS_QA_WINDOW_TITLE,
  redact,
  repoDir,
  run,
} from "./macos-qa-profile.mjs";

const windowListSource = String.raw`
import CoreGraphics
import Foundation

let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
let info = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] ?? []
let windows = info.compactMap { item -> [String: Any]? in
    let owner = item[kCGWindowOwnerName as String] as? String ?? ""
    let name = item[kCGWindowName as String] as? String ?? ""
    guard owner.contains("Clark Code") || name.contains("Clark Code") else { return nil }
    return [
        "id": item[kCGWindowNumber as String] as? Int ?? 0,
        "owner": owner,
        "name": name,
        "bounds": item[kCGWindowBounds as String] as? [String: Any] ?? [:],
    ]
}
let data = try! JSONSerialization.data(withJSONObject: windows)
print(String(data: data, encoding: .utf8)!)
`;

const ocrSource = String.raw`
import Foundation
import ImageIO
import Vision

guard CommandLine.arguments.count == 2 else { fatalError("expected one image") }
let url = URL(fileURLWithPath: CommandLine.arguments[1]) as CFURL
guard
    let source = CGImageSourceCreateWithURL(url, nil),
    let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
else { fatalError("cannot load image") }
let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
let handler = VNImageRequestHandler(cgImage: image)
try handler.perform([request])
let lines = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
print(lines.joined(separator: "\n"))
`;

function sleep(milliseconds) {
  if (milliseconds > 0) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
  }
}

function listClarkWindows() {
  const completed = run("swift", ["-"], {
    input: windowListSource,
    timeout_ms: 30_000,
  });
  if (!completed.ok) {
    throw new Error(`cannot inventory Clark windows: ${redact(completed.stderr)}`);
  }
  const parsed = JSON.parse(completed.stdout.trim());
  return parsed.filter(
    (window) => Number.isInteger(window.id) && window.id > 0,
  );
}

function imageStatistics(imagePath) {
  const dimensions = run("magick", ["identify", "-format", "%w %h", imagePath]);
  if (!dimensions.ok) throw new Error("cannot inspect macOS QA screenshot");
  const [width, height] = dimensions.stdout.trim().split(/\s+/).map(Number);
  const measured = run("magick", [
    imagePath,
    "-alpha",
    "off",
    "-colorspace",
    "Gray",
    "-format",
    "%[fx:mean] %[fx:standard_deviation]",
    "info:",
  ]);
  if (!measured.ok) throw new Error("cannot measure macOS QA screenshot");
  const [mean, standardDeviation] = measured.stdout.trim().split(/\s+/).map(Number);
  return {
    width,
    height,
    mean,
    standard_deviation: standardDeviation,
  };
}

export function captureMacosProductWindow(outputDir) {
  const evidenceDir = path.join(outputDir, "evidence");
  mkdirSync(evidenceDir, { recursive: true, mode: 0o700 });
  chmodSync(evidenceDir, 0o700);
  const screenshotPath = path.join(evidenceDir, "macos.png");
  let matches = [];
  for (let attempt = 0; attempt < 20; attempt += 1) {
    matches = listClarkWindows().filter(
      (window) => window.name === MACOS_QA_WINDOW_TITLE,
    );
    if (matches.length === 1) {
      const captured = run(
        "/usr/sbin/screencapture",
        ["-x", "-o", "-l", String(matches[0].id), screenshotPath],
        { timeout_ms: 30_000 },
      );
      if (captured.ok) break;
    }
    sleep(1_000);
  }
  if (matches.length !== 1 || !existsSync(screenshotPath)) {
    return {
      status: "failed",
      window_visible: false,
      error: `expected one on-screen window titled ${MACOS_QA_WINDOW_TITLE}`,
    };
  }
  chmodSync(screenshotPath, 0o600);
  const image = imageStatistics(screenshotPath);
  const graphical = (
    image.width >= 900
    && image.height >= 600
    && image.mean >= 0.03
    && image.standard_deviation >= 0.015
  );
  const recognized = run("swift", ["-", screenshotPath], {
    input: ocrSource,
    timeout_ms: 120_000,
  });
  const text = recognized.stdout.toLowerCase();
  const markers = {
    brand_visible: text.includes("clark code"),
    workspace_visible: text.includes("new session"),
    project_visible: text.includes("clarkcodeqa"),
    model_visible: text.includes("free"),
    execution_control_visible: text.includes("execute"),
    sign_in_absent: !text.includes("continue with google"),
  };
  const passed = (
    graphical
    && recognized.ok
    && Object.values(markers).every(Boolean)
  );
  return {
    status: passed ? "passed" : "failed",
    window_visible: true,
    window_match_count: matches.length,
    capture_transport: "macos_window_id",
    screenshot: path.relative(repoDir, screenshotPath),
    screenshot_sha256: createHash("sha256")
      .update(readFileSync(screenshotPath))
      .digest("hex"),
    image,
    visual_contract: {
      status: passed ? "passed" : "failed",
      transport: "macos_vision_ocr",
      markers,
      recognized_text_recorded: false,
    },
  };
}
