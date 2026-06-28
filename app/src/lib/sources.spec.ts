import { describe, expect, it } from "vitest";
import { extractSources } from "./sources";

describe("extractSources", () => {
  it("returns nothing when there are no links", () => {
    expect(extractSources("plain prose, no urls here")).toEqual([]);
  });

  it("labels a link by its hostname, stripping www.", () => {
    const got = extractSources("see https://www.example.com/docs/page for details");
    expect(got).toEqual([{ url: "https://www.example.com/docs/page", label: "example.com" }]);
  });

  it("trims punctuation that clings to a url in prose", () => {
    const [s] = extractSources("docs at https://docs.rs/clap/latest/clap/.");
    expect(s.url).toBe("https://docs.rs/clap/latest/clap/");
  });

  it("dedupes by host so deep links collapse to one chip per site", () => {
    const text =
      "See https://docs.rs/clap/latest/clap/ and the tutorial at " +
      "https://docs.rs/clap/latest/clap/_derive/_tutorial/index.html plus https://serde.rs/.";
    const got = extractSources(text);
    expect(got.map((s) => s.label)).toEqual(["docs.rs", "serde.rs"]);
    // keeps the first-mentioned url for the host
    expect(got[0].url).toBe("https://docs.rs/clap/latest/clap/");
  });

  it("honours the cap on distinct hosts", () => {
    const text = ["a", "b", "c"].map((h) => `https://${h}.com/x`).join(" ");
    expect(extractSources(text, 2)).toHaveLength(2);
  });

  it("skips unparseable url-ish tokens", () => {
    expect(extractSources("https:// not a url")).toEqual([]);
  });
});
