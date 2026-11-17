import { describe, expect, it } from "vitest";
import { SIDEBAR_RAIL_WIDTH } from "./sidebar";
import styles from "./styles.css?raw";

describe("the rail's width", () => {
  it("is the first column of the app's grid", () => {
    expect(styles).toMatch(new RegExp(`\\.app \\{[^}]*grid-template-columns: ${SIDEBAR_RAIL_WIDTH}px 1fr;`));
  });
});
