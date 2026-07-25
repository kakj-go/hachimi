import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

const wcagTags = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"];

for (const theme of ["light", "dark"] as const) {
  for (const locale of ["zh-CN", "en-US"] as const) {
    for (const zoom of [100, 125, 150] as const) {
      test(`settings ${theme} ${locale} ${zoom}%`, async ({ page }) => {
        await page.goto(
          `/iframe.html?id=examples-settings--default&globals=theme:${theme};locale:${locale};zoom:${zoom}`,
        );
        await expect(page.locator("main")).toBeVisible();
        await expect(page).toHaveScreenshot(`settings-${theme}-${locale}-${zoom}.png`, {
          animations: "disabled",
        });
      });
    }
  }
}

test("button and text field expose keyboard focus", async ({ page }) => {
  await page.goto("/iframe.html?id=components-button--default");
  await page.locator("body").click({ position: { x: 1, y: 1 } });
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: "Button" })).toBeFocused();
  expect((await new AxeBuilder({ page }).withTags(wcagTags).analyze()).violations).toEqual([]);

  await page.goto("/iframe.html?id=components-forms--text-input");
  await page.locator("body").click({ position: { x: 1, y: 1 } });
  await page.keyboard.press("Tab");
  await expect(page.getByRole("textbox", { name: "API Base URL" })).toBeFocused();
  expect((await new AxeBuilder({ page }).withTags(wcagTags).analyze()).violations).toEqual([]);
});

test("context menu exposes the enabled Workbench item", async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 480 });
  await page.goto("/iframe.html?id=components-contextmenu--default");
  await page.getByText("Right-click this area").click({ button: "right" });
  const workbench = page.getByText("Workbench", { exact: true });
  const menu = page.locator('[data-component="menu-content"]');
  await expect(workbench).toBeVisible();
  await expect(workbench.locator("..")).toHaveAttribute("aria-disabled", "false");
  expect(await menu.evaluate((element) => element.scrollHeight <= element.clientHeight)).toBe(true);
  expect(
    (
      await new AxeBuilder({ page })
        .withTags(wcagTags)
        .disableRules(["aria-required-children"])
        .analyze()
    ).violations,
  ).toEqual([]);
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(menu).toBeHidden();
  await page.getByText("Right-click this area").click({ button: "right" });
  await workbench.click();
  await expect(workbench).toBeHidden();
});

test("dialog supports focus and Escape", async ({ page }) => {
  await page.goto("/iframe.html?id=components-dialog--default");
  await page.getByRole("button", { name: "Open dialog" }).click();
  const dialog = page.getByRole("dialog");
  const close = page.locator('[data-component="dialog-close"]');
  await expect(dialog).toBeVisible();
  await expect(close).toBeVisible();
  const dialogBox = await dialog.boundingBox();
  const closeBox = await close.boundingBox();
  expect(dialogBox).not.toBeNull();
  expect(closeBox).not.toBeNull();
  expect(closeBox!.x).toBeGreaterThan(dialogBox!.x + dialogBox!.width - 64);
  expect(closeBox!.y).toBeLessThan(dialogBox!.y + 64);
  expect((await new AxeBuilder({ page }).withTags(wcagTags).analyze()).violations).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});

for (const route of ["home", "general", "appearance", "llm", "avatar", "voice"] as const) {
  for (const theme of ["light", "dark"] as const) {
    for (const locale of ["zh-CN", "en-US"] as const) {
      for (const zoom of [100, 125, 150] as const) {
        test(`workbench ${route} ${theme} ${locale} ${zoom}%`, async ({ page }) => {
          await page.goto(
            `/iframe.html?id=examples-workbench--${route}&globals=theme:${theme};locale:${locale};zoom:${zoom}`,
          );
          await expect(page.getByText("Hachimi", { exact: true }).first()).toBeVisible();
          await expect(page).toHaveScreenshot(`workbench-${route}-${theme}-${locale}-${zoom}.png`, {
            animations: "disabled",
          });
        });
      }
    }
  }
}

for (const state of ["fallback", "glb-loading", "input", "reply", "muted", "error"] as const) {
  test(`pet ${state}`, async ({ page }) => {
    await page.goto(
      `/iframe.html?id=examples-pet--${state}&globals=theme:dark;locale:zh-CN;zoom:100`,
    );
    if (state === "input") {
      await expect(page.getByRole("button", { name: "消息" })).toHaveCount(0);
      await expect(page.getByRole("button", { name: "语音输入" })).toBeVisible();
    } else {
      await expect(page.getByRole("button", { name: "消息" })).toBeVisible();
    }
    await expect(page).toHaveScreenshot(`pet-${state}.png`, { animations: "disabled" });
  });
}
