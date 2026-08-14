import { describe, expect, it } from "vitest";

import { guidedSpecPrompt, specGuidance } from "./specGuidance";

describe("specGuidance", () => {
  it("does not mistake the blank starter prompts for settled decisions", () => {
    const report = specGuidance(`# New idea

## Overview

Describe the feature in your own words.

## Users

- Who should use this?`);

    expect(report.clear).toBe(0);
    expect(report.current.id).toBe("purpose");
  });

  it("advances to the first genuinely uncovered decision", () => {
    const report = specGuidance(`# Calm handoff

## Problem and outcome

Support teams lose the context behind a customer request. The feature should preserve the original intent and make the next action obvious to everyone involved.

## Users and roles

The first audience is a support lead who receives a request and needs to hand it to an engineer without translating the customer's language.`);

    expect(report.clear).toBe(2);
    expect(report.current.id).toBe("journey");
  });

  it("wraps a plain-language answer as a scoped document decision", () => {
    const question = specGuidance("").current;
    const prompt = guidedSpecPrompt(question, "People should know what to do next without training.");

    expect(prompt).toContain("People should know what to do next without training.");
    expect(prompt).toContain("preserving unrelated content");
    expect(prompt).toContain("Do not implement the feature");
  });
});
