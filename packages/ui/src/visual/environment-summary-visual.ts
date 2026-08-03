import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Locator, type Page } from "@playwright/test";

type InstallTauriMocks = (
  page: Page,
  withComposerData?: boolean,
  schedulerEnabled?: boolean,
  withSessionData?: boolean,
  themeMode?: "light" | "dark" | "system",
) => Promise<void>;

export function installEnvironmentSummaryVisualTests(installTauriMocks: InstallTauriMocks) {
  test("workbench repairs an oversized persisted project pane without displacing the session", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 2048, height: 1104 });
    await installTauriMocks(page, true, false, true);
    await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
    await page.evaluate(() => {
      window.localStorage.setItem(
        "hachimi.workbench.layout.v2",
        JSON.stringify({
          summaryPinned: false,
          bottomPanelOpen: false,
          sidebarVisible: false,
          projectSidebarWidth: 1_450,
          inspectorWidth: 380,
          bottomPanelHeight: 250,
        }),
      );
    });
    await page.reload();

    const layout = await page.locator(".home-layout").evaluate((element) => {
      const sidebar = element.querySelector(".project-sidebar")!.getBoundingClientRect();
      const main = element.querySelector(".home-main")!.getBoundingClientRect();
      const persisted = JSON.parse(
        window.localStorage.getItem("hachimi.workbench.layout.v2") ?? "{}",
      ) as { projectSidebarWidth?: number };
      return {
        sidebarWidth: sidebar.width,
        mainLeft: main.left,
        mainWidth: main.width,
        persistedWidth: persisted.projectSidebarWidth,
      };
    });

    expect(layout.sidebarWidth).toBeLessThanOrEqual(480);
    expect(layout.mainLeft).toBeLessThanOrEqual(485);
    expect(layout.mainWidth).toBeGreaterThan(1_500);
    expect(layout.persistedWidth).toBe(480);
    await expect(page.locator("html")).toHaveJSProperty("scrollWidth", 2048);
  });

  test("production active Agent session uses the shared workflow and workspace contract", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.clock.setFixedTime(new Date("2026-07-26T15:00:00.000Z"));
    await installTauriMocks(page, true, false, true);
    await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
    await page.getByTestId("session-select-session-ui-unification").click();

    await expect(page.getByTestId("workbench-conversation-title")).toContainText(
      "统一前端视觉规范与组件样式",
    );
    await expect(page.locator('[data-component="agent-message"]')).toHaveCount(2);
    await expect(page.locator(".tool-activity-batch")).toHaveCount(1);
    await expect(page.locator('[data-component="approval"]')).toBeVisible();
    await expect(page.locator('[data-component="composer"]')).toBeHidden();
    await expect(page.locator(".workbench-toolbar")).toBeVisible();
    await expect(page.locator(".workbench-inspector")).toHaveCount(0);
    await page.getByTestId("workbench-toggle-inspector").click();
    await expect(page.getByTestId("workbench-resource-menu")).toContainText("审阅");
    await expect(page.getByTestId("workbench-resource-menu")).toContainText("浏览器");
    await expect(page.locator('.workbench-inspector[data-resource="tools"]')).toBeVisible();
    await page.getByTestId("workbench-pin-summary").click();
    const summary = page.locator(".workbench-pinned-summary .environment-summary");
    await expect(summary).toContainText("环境信息");
    await expect(summary).toContainText("learn.chatgpt.com");
    await expect(summary).not.toContainText("拉取请求");
    await expect(page).toHaveScreenshot("production-agent-summary-1440x900.png", {
      animations: "disabled",
    });

    await page.getByTestId("workbench-summary-location").click();
    await expect(page.locator(".environment-location-popover")).toBeVisible();
    await expect(page).toHaveScreenshot("production-agent-location-menu-1440x900.png", {
      animations: "disabled",
    });
    await page.keyboard.press("Escape");
    await expect(page.locator(".environment-location-popover")).toHaveCount(0);

    await page.getByTestId("workbench-git-branch-trigger").click();
    await expect(page.locator(".workbench-git-popover.branch")).toBeVisible();
    await expect(page).toHaveScreenshot("production-agent-branch-menu-1440x900.png", {
      animations: "disabled",
    });
    await page.keyboard.press("Escape");
    await expect(page.locator(".workbench-git-popover.branch")).toHaveCount(0);

    await page.getByTestId("workbench-git-commit-trigger").click();
    await expect(page.locator(".workbench-git-popover.commit")).toBeVisible();
    await expect(page.getByTestId("workbench-git-commit-and-push")).toBeVisible();
    await expect(page).toHaveScreenshot("production-agent-commit-menu-1440x900.png", {
      animations: "disabled",
    });
    await page.keyboard.press("Escape");
    await expect(page.locator(".workbench-git-popover.commit")).toHaveCount(0);

    await page.getByTestId("workbench-git-compare").click();
    await expect(page.locator('[data-component="workspace"][data-mode="review"]')).toBeVisible();
    await expect(page.getByTestId("workspace-diff-branch-select")).toBeVisible();
    await expect(page).toHaveScreenshot("production-agent-branch-diff-1440x900.png", {
      animations: "disabled",
    });
    await expect(page.locator("html")).toHaveJSProperty("scrollWidth", 1440);

    const result = await new AxeBuilder({ page })
      .include(".home-main")
      .withTags(["wcag2a", "wcag2aa"])
      .disableRules(["nested-interactive"])
      .analyze();
    expect(result.violations).toEqual([]);
    await expect(page).toHaveScreenshot("production-agent-session-1440x900.png", {
      animations: "disabled",
    });
  });

  for (const viewport of [
    { name: "1440x900", width: 1440, height: 900 },
    { name: "960x700", width: 960, height: 700 },
    { name: "720x640", width: 720, height: 640 },
  ] as const) {
    for (const theme of ["light", "dark"] as const) {
      if (viewport.name === "1440x900" && theme === "dark") continue;
      test(`environment summary remains responsive at ${viewport.name} in ${theme}`, async ({
        page,
      }) => {
        await page.setViewportSize({ width: viewport.width, height: viewport.height });
        await page.clock.setFixedTime(new Date("2026-07-26T15:00:00.000Z"));
        await installTauriMocks(page, true, false, true, theme);
        await page.goto("http://127.0.0.1:1420/workbench.html?route=home");
        if (viewport.width <= 720) {
          await page.getByRole("button", { name: "Sidebar" }).click();
          await expect(page.locator(".project-sidebar")).toBeVisible();
        }
        await page.getByTestId("session-select-session-ui-unification").click();
        if (viewport.width <= 720) {
          await page.getByRole("button", { name: "Sidebar" }).click();
          await expect(page.locator(".project-sidebar")).toBeHidden();
        }
        await page.getByTestId("workbench-pin-summary").click();
        const summary = page.locator(".workbench-pinned-summary .environment-summary");
        await expect(summary).toBeVisible();
        await expect(summary).toContainText("Git worktrees");
        await expect(page.locator("html")).toHaveJSProperty("scrollWidth", viewport.width);
        await expect(page).toHaveScreenshot(
          `production-environment-summary-${viewport.name}-${theme}.png`,
          { animations: "disabled" },
        );

        for (const row of [
          page.getByTestId("workbench-git-commit-trigger"),
          page.getByTestId("workbench-git-compare"),
          page.getByTestId("workbench-summary-browser-activity"),
          page.getByTestId("workbench-summary-sources-all"),
          summary.getByTitle("Git worktrees"),
        ]) {
          await expectSummaryRowReachable(row);
        }
      });
    }
  }
}

async function expectSummaryRowReachable(row: Locator) {
  await row.scrollIntoViewIfNeeded();
  await expect(row).toBeVisible();
  const insideVisibleSummary = await row.evaluate((element) => {
    const rowRect = element.getBoundingClientRect();
    const summaryRect = element.closest(".workbench-pinned-summary")?.getBoundingClientRect();
    const columnRect = element.closest(".session-primary-column")?.getBoundingClientRect();
    if (!summaryRect || !columnRect) return false;
    const visibleTop = Math.max(0, summaryRect.top, columnRect.top);
    const visibleBottom = Math.min(window.innerHeight, summaryRect.bottom, columnRect.bottom);
    return rowRect.top >= visibleTop - 1 && rowRect.bottom <= visibleBottom + 1;
  });
  expect(insideVisibleSummary).toBe(true);
}
