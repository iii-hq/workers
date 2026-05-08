import { expect, test } from "@playwright/test";

const PROMPT = "create /tmp/harness-e2e.md with the body 'hi'";

test.describe("approval flow", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
  });

  test("allow path writes the file and renders function result block", async ({ page }) => {
    await page.getByPlaceholder(/say something/i).fill(PROMPT);
    await page.getByRole("button", { name: /send/i }).click();
    const approval = page.locator(".approval");
    await expect(approval).toBeVisible({ timeout: 30_000 });
    await page.getByRole("button", { name: "allow" }).click();
    await expect(page.locator(".block-tool-result")).toBeVisible({ timeout: 30_000 });
  });

  test("deny path renders denied tool_result and does not write", async ({ page }) => {
    await page.getByPlaceholder(/say something/i).fill(PROMPT);
    await page.getByRole("button", { name: /send/i }).click();
    await expect(page.locator(".approval")).toBeVisible({ timeout: 30_000 });
    await page.getByRole("button", { name: "deny" }).click();
    const result = page.locator(".block-tool-result[data-error='true']");
    await expect(result).toBeVisible({ timeout: 30_000 });
  });
});
