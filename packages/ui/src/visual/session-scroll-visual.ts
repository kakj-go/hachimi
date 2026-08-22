import { expect, test, type Page } from "@playwright/test";

type InstallTauriMocks = (
  page: Page,
  withComposerData?: boolean,
  schedulerEnabled?: boolean,
  withSessionData?: boolean,
  gateMode?: "approval" | "plan" | "user_input",
) => Promise<void>;

export function installSessionScrollVisualTest(installTauriMocks: InstallTauriMocks) {
  test("plan mode uses a removable composer chip", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await installTauriMocks(page, true);
    await page.goto("http://127.0.0.1:1420/workbench.html?route=home");

    await page.getByTestId("workbench-task-options").click();
    const menuItem = page.getByTestId("workbench-plan-mode");
    await expect(menuItem).toHaveAttribute("aria-pressed", "false");
    await menuItem.click();

    const permission = page.getByTestId("workbench-permission-profile");
    const chip = page.getByTestId("workbench-plan-mode-chip");
    await expect(page.getByTestId("workbench-options-popover")).toHaveCount(0);
    await expect(chip).toBeVisible();
    await expect(page.locator(".plan-mode-banner")).toHaveCount(0);
    const [permissionBox, chipBox] = await Promise.all([
      permission.boundingBox(),
      chip.boundingBox(),
    ]);
    expect(chipBox!.x).toBeGreaterThan(permissionBox!.x + permissionBox!.width);

    const defaultIcon = chip.locator(".composer-plan-mode-default-icon");
    const removeIcon = chip.locator(".composer-plan-mode-remove-icon");
    await expect(defaultIcon).toHaveCSS("opacity", "1");
    await expect(removeIcon).toHaveCSS("opacity", "0");
    const chipBoxBeforeHover = await chip.boundingBox();
    await chip.hover();
    await expect(defaultIcon).toHaveCSS("opacity", "0");
    await expect(removeIcon).toHaveCSS("opacity", "1");
    await expect(page.locator('[data-component="tooltip-content"]')).toContainText("关闭计划模式");
    expect(await chip.boundingBox()).toEqual(chipBoxBeforeHover);
    await page.mouse.move(0, 0);
    await chip.focus();
    await page.keyboard.press("Tab");
    await page.keyboard.press("Shift+Tab");
    await expect(chip).toBeFocused();
    await expect(defaultIcon).toHaveCSS("opacity", "0");
    await expect(removeIcon).toHaveCSS("opacity", "1");

    await page.setViewportSize({ width: 720, height: 640 });
    const [optionsBox, sendBox] = await Promise.all([
      page.locator(".composer-options").boundingBox(),
      page.getByTestId("workbench-start-task").boundingBox(),
    ]);
    expect(optionsBox!.x + optionsBox!.width).toBeLessThanOrEqual(sendBox!.x);
    await expect(page.locator("html")).toHaveJSProperty("scrollWidth", 720);

    await expect(page.locator(".project-sidebar")).toBeHidden();
    await chip.click();
    await expect(chip).toHaveCount(0);
    await page.getByTestId("workbench-task-options").click();
    await expect(page.getByTestId("workbench-plan-mode")).toHaveAttribute("aria-pressed", "false");
  });

  test("approval details and decisions remain visible on a narrow viewport", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await installTauriMocks(page, true, false, true);
    await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
    await page.getByTestId("project-select-project-hachimi").click();
    await page.getByTestId("session-select-session-ui-unification").click();
    await page.setViewportSize({ width: 720, height: 640 });

    await expect(page.getByTestId("approval-request-details")).toBeVisible();
    await expect(page.getByTestId("workbench-deny-approval")).toBeVisible();
    await expect(page.getByTestId("workbench-approve-once")).toBeVisible();
    await expect(page.locator("html")).toHaveJSProperty("scrollWidth", 720);
  });

  test("session viewport owns scrolling and exposes follow-latest behavior", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await installTauriMocks(page, true, false, true);
    await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
    await page.getByTestId("project-select-project-hachimi").click();
    await page.getByTestId("session-select-session-ui-unification").click();
    await page.locator(".timeline-items").evaluate((element) => {
      for (let index = 0; index < 60; index += 1) {
        const item = document.createElement("article");
        item.className = "timeline-narration";
        item.textContent = `stream fixture ${index}: ${"content ".repeat(18)}`;
        element.append(item);
      }
    });

    const viewport = page.locator(".session-scroll-viewport");
    await expect
      .poll(() => viewport.evaluate((element) => element.scrollHeight > element.clientHeight))
      .toBe(true);
    await expect
      .poll(() =>
        viewport.evaluate(
          (element) => element.scrollHeight - element.scrollTop - element.clientHeight,
        ),
      )
      .toBeLessThan(32);
    const scrollOwners = await page
      .locator(".session-scroll-viewport, .session-timeline, .timeline-items")
      .evaluateAll(
        (elements) =>
          elements.filter((element) =>
            ["auto", "scroll"].includes(getComputedStyle(element).overflowY),
          ).length,
      );
    expect(scrollOwners).toBe(1);
    const [surfaceBox, viewportBox] = await Promise.all([
      page.locator(".home-agent-surface").boundingBox(),
      viewport.boundingBox(),
    ]);
    expect(
      Math.abs(surfaceBox!.x + surfaceBox!.width - (viewportBox!.x + viewportBox!.width)),
    ).toBe(0);

    await viewport.evaluate((element) => {
      element.scrollTop = 0;
      element.dispatchEvent(new Event("scroll"));
    });
    await expect(page.locator(".timeline-jump-bottom")).toBeVisible();
    const [jumpBox, composerBox] = await Promise.all([
      page.locator(".timeline-jump-bottom").boundingBox(),
      page.locator(".composer-wrap").boundingBox(),
    ]);
    expect(composerBox!.y - (jumpBox!.y + jumpBox!.height)).toBeGreaterThanOrEqual(0);
    expect(composerBox!.y - (jumpBox!.y + jumpBox!.height)).toBeLessThanOrEqual(8);
    const beforeAppend = await viewport.evaluate((element) => element.scrollTop);
    await page.locator(".timeline-items").evaluate((element) => {
      const item = document.createElement("article");
      item.className = "timeline-narration";
      item.textContent = "content added while the reader is away from the bottom";
      element.append(item);
    });
    await expect.poll(() => viewport.evaluate((element) => element.scrollTop)).toBe(beforeAppend);

    await page.locator(".timeline-jump-bottom").click();
    await expect
      .poll(() =>
        viewport.evaluate(
          (element) => element.scrollHeight - element.scrollTop - element.clientHeight,
        ),
      )
      .toBeLessThan(32);
    await expect(page.locator(".timeline-jump-bottom")).toHaveCount(0);

    await page.locator(".timeline-items").evaluate((element) => {
      const item = document.createElement("article");
      item.className = "timeline-narration";
      item.textContent = "content added while following the latest output";
      element.append(item);
    });
    await expect
      .poll(() =>
        viewport.evaluate(
          (element) => element.scrollHeight - element.scrollTop - element.clientHeight,
        ),
      )
      .toBeLessThan(32);

    const rightBeforeInspector = await rightEdge(viewport);
    await page.getByTestId("workbench-toggle-inspector").click();
    await expect(page.locator(".workbench-inspector")).toBeVisible();
    expect(await rightEdge(viewport)).toBeLessThan(rightBeforeInspector);
  });
}

export async function assertPermissionTones(
  page: Page,
  permissionPopover: ReturnType<Page["getByTestId"]>,
) {
  await expect(permissionPopover).toHaveCSS("width", "380px");
  const readOnly = page.getByTestId("workbench-permission-read_only");
  const writable = page.getByTestId("workbench-permission-writable");
  const fullAccess = page.getByTestId("workbench-permission-full_access");
  await expect(readOnly).toHaveAttribute("data-tone", "neutral");
  await expect(writable).toHaveAttribute("data-tone", "recommended");
  await expect(writable).toHaveAttribute("aria-pressed", "true");
  await expect(fullAccess).toHaveAttribute("data-tone", "danger");
  const colors = await permissionPopover.locator(".composer-popover-row").evaluateAll((rows) =>
    rows.map((row) => ({
      color: getComputedStyle(row).color,
      background: getComputedStyle(row).backgroundColor,
    })),
  );
  expect(colors[2]?.color).not.toBe(colors[0]?.color);
  expect(colors[1]?.background).not.toBe(colors[0]?.background);
}

async function rightEdge(locator: ReturnType<Page["locator"]>) {
  const box = await locator.boundingBox();
  return box!.x + box!.width;
}
