import { expect, test } from "@playwright/test";
import { existsSync, rmSync, statSync } from "node:fs";

const MODEL = "claude-haiku-4-5";
const HARNESS_CALL_URL = "http://127.0.0.1:3111/harness/call";
type ModelRow = { id?: string };

function isDirectory(path: string): boolean {
  return existsSync(path) && statSync(path).isDirectory();
}

test.describe("approval flow", () => {
  test.beforeEach(async ({ page, request }) => {
    await expect
      .poll(
        async () => {
          const response = await request.post(HARNESS_CALL_URL, {
            data: { function_id: "models::list", payload: {} },
          });
          if (!response.ok()) return false;
          const body = await response.json();
          return body.models?.some((model: ModelRow) => model.id === MODEL) === true;
        },
        { timeout: 30_000 },
      )
      .toBe(true);

    await page.goto("/");
    await expect(page.locator(".model-select")).toContainText(MODEL, { timeout: 30_000 });
    await page.locator(".model-select").selectOption(MODEL);
  });

  test("allow path runs the approved function and renders function result block", async ({
    page,
  }) => {
    const path = `/tmp/harness-e2e-allow-${test.info().parallelIndex}`;
    rmSync(path, { force: true, recursive: true });

    await page.locator(".composer-input").fill(
      `Use shell::fs::mkdir to create the directory ${path}.`,
    );
    await page.getByRole("button", { name: /send/i }).click();
    const approval = page.locator(".approval");
    await expect(approval).toBeVisible({ timeout: 90_000 });
    await approval.locator(".approval-allow").click();
    await expect(page.locator(".block-tool-result").filter({ hasText: "created" })).toBeVisible({
      timeout: 90_000,
    });
    await expect.poll(() => isDirectory(path), { timeout: 90_000 }).toBe(true);
  });

  test("deny path renders denied tool_result and does not run function", async ({ page }) => {
    const path = `/tmp/harness-e2e-deny-${test.info().parallelIndex}`;
    rmSync(path, { force: true, recursive: true });

    await page.locator(".composer-input").fill(
      `Use shell::fs::mkdir to create the directory ${path}.`,
    );
    await page.getByRole("button", { name: /send/i }).click();
    const approval = page.locator(".approval");
    await expect(approval).toBeVisible({ timeout: 90_000 });
    await approval.locator(".approval-deny").click();
    const result = page.locator(".block-tool-result[data-error='true']");
    await expect(result).toBeVisible({ timeout: 90_000 });
    expect(existsSync(path)).toBe(false);
  });
});
