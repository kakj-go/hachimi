import { expect, test, type Page } from "@playwright/test";

type InstallTauriMocks = (page: Page) => Promise<void>;

export function installSettingsLayoutVisualTests(installTauriMocks: InstallTauriMocks) {
  test("model selects align with text inputs", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await installTauriMocks(page);
    await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/llm");

    const modelNameBox = await page.getByRole("textbox", { name: "模型名称" }).boundingBox();
    const structuredOutputBox = await page
      .locator('[data-component="settings-row"]', { hasText: "结构化输出" })
      .locator('[data-component="select"]')
      .boundingBox();
    expect(modelNameBox).not.toBeNull();
    expect(structuredOutputBox).not.toBeNull();
    expect(Math.abs(modelNameBox!.x - structuredOutputBox!.x)).toBeLessThan(1);
    expect(Math.abs(modelNameBox!.width - structuredOutputBox!.width)).toBeLessThan(1);
  });

  test("light and dark theme profile controls stay compact and inline", async ({ page }) => {
    await page.setViewportSize({ width: 1855, height: 1343 });
    await installTauriMocks(page);
    await page.goto("http://127.0.0.1:1420/workbench.html?route=settings/appearance");

    const layouts = await page.locator(".theme-profile-toolbar").evaluateAll((toolbars) =>
      toolbars.map((toolbar) => {
        const labelBox = toolbar
          .querySelector('[data-component="form-label"]')!
          .getBoundingClientRect();
        const triggerBox = toolbar
          .querySelector('[data-component="select-trigger"]')!
          .getBoundingClientRect();
        return {
          triggerWidth: triggerBox.width,
          centerDelta: Math.abs(
            labelBox.y + labelBox.height / 2 - (triggerBox.y + triggerBox.height / 2),
          ),
        };
      }),
    );
    expect(layouts).toHaveLength(2);
    for (const layout of layouts) {
      expect(layout.triggerWidth).toBeLessThanOrEqual(340);
      expect(layout.centerDelta).toBeLessThan(1);
    }
  });
}
