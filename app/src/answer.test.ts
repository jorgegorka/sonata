import { describe, expect, it } from "vitest";
import { answerBody } from "./answer";

describe("answerBody", () => {
  it("is the option alone when the box is empty", () => {
    expect(answerBody("Approve", "")).toBe("Approve");
    expect(answerBody("Approve", "  ")).toBe("Approve");
  });
  it("puts the trimmed text after the option", () => {
    expect(answerBody("Approve", " use X \n")).toBe("Approve\n\nuse X");
  });
});
