import { expect, type Page } from "@playwright/test";

const VIEWPORTS = [
  { width: 960, height: 700 },
  { width: 1280, height: 800 },
  { width: 1855, height: 1080 },
] as const;

export async function assertPlatformIntegrationsVisualMatrix(page: Page) {
  for (const scheme of ["dark", "light"] as const) {
    if (scheme === "light") {
      await page.locator(".settings-nav").getByRole("button", { name: "外观" }).click();
      await page.getByRole("radio", { name: /奶油手帐/ }).click();
      await page.locator(".settings-nav").getByRole("button", { name: "平台集成" }).click();
    }
    await expect(page.locator("html")).toHaveAttribute("data-color-scheme", scheme);
    for (const viewport of VIEWPORTS) {
      await page.setViewportSize(viewport);
      await page.mouse.move(viewport.width - 8, 8);
      await expect(page).toHaveScreenshot(
        `production-platform-integrations-${scheme}-${viewport.width}x${viewport.height}.png`,
        { animations: "disabled", fullPage: true },
      );
    }
    for (const scale of [125, 150]) {
      await page.setViewportSize({ width: 1280, height: 800 });
      await page.locator("html").evaluate((root, percent) => {
        root.style.zoom = `${percent}%`;
      }, scale);
      await page.mouse.move(1272, 8);
      await expect(page).toHaveScreenshot(
        `production-platform-integrations-${scheme}-1280x800-${scale}pct.png`,
        { animations: "disabled", fullPage: true },
      );
    }
    await page.locator("html").evaluate((root) => {
      root.style.zoom = "100%";
    });
    await page.setViewportSize({ width: 1280, height: 800 });
    await page
      .getByTestId("integration-provider-dingtalk")
      .getByRole("button", { name: "连接账户" })
      .first()
      .click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await expect(page).toHaveScreenshot(
      `production-platform-integrations-${scheme}-account-dialog-1280x800.png`,
      { animations: "disabled", fullPage: true },
    );
    await page.getByRole("button", { name: "关闭" }).click();
    await page.getByRole("tab", { name: "企微自建应用" }).click();
    await page
      .getByTestId("integration-provider-wecom_app")
      .getByRole("button", { name: "策略" })
      .click();
    await expect(page.getByRole("dialog", { name: "消息策略" })).toBeVisible();
    await expect(page).toHaveScreenshot(
      `production-platform-integrations-${scheme}-policy-dialog-1280x800.png`,
      { animations: "disabled", fullPage: true },
    );
    await page.getByRole("button", { name: "关闭" }).click();
    await page.getByRole("tab", { name: "钉钉" }).click();
  }
}
