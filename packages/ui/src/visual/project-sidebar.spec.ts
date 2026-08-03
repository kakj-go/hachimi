import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const componentCss = readFileSync(resolve(import.meta.dirname, "../styles/components.css"), "utf8");
const workbenchCss = readFileSync(
  resolve(import.meta.dirname, "../../../workbench/src/workbench.css"),
  "utf8",
);

test("project and session rows reveal hover actions without resting backgrounds", async ({
  page,
}) => {
  await page.setContent(`
    <style>
      :root {
        --color-text-base: rgb(240 240 240);
        --color-text-muted: rgb(180 180 180);
        --color-text-faint: rgb(140 140 140);
        --color-overlay-hover: rgb(255 255 255 / 8%);
        --duration-fast: 0ms;
        --ease-standard: linear;
        --space-1: 4px;
        --radius-md: 6px;
        --font-xs: 12px;
      }
      ${componentCss}
      ${workbenchCss}
    </style>
    <div class="project-row-shell">
      <button data-component="button" class="project-row" aria-current="page">Project</button>
      <div class="project-row-actions">
        <button data-component="dropdown-trigger" aria-label="Project actions">...</button>
        <button data-component="button" class="project-new-task" aria-label="New task">+</button>
      </div>
    </div>
    <div class="project-sessions">
      <div class="session-row-shell selected">
        <button data-component="button" aria-current="page">Session</button>
        <button data-component="dropdown-trigger" aria-label="Session actions">...</button>
      </div>
    </div>
  `);

  const transparent = "rgba(0, 0, 0, 0)";
  const noShadow = "none";
  const projectShell = page.locator(".project-row-shell");
  const project = page.locator(".project-row");
  const projectActions = projectShell.locator(".project-row-actions");
  const projectMenu = projectShell.locator('[aria-label="Project actions"]');
  const projectNewTask = projectShell.locator('[aria-label="New task"]');
  const sessionShell = page.locator(".session-row-shell");
  const session = sessionShell.locator('[data-component="button"]');
  const menu = sessionShell.locator('[data-component="dropdown-trigger"]');

  await expect(project).toHaveCSS("background-color", transparent);
  await expect(project).toHaveCSS("box-shadow", noShadow);
  await expect(projectShell).toHaveCSS("background-color", transparent);
  await expect(projectActions).toHaveCSS("opacity", "0");
  await expect(session).toHaveCSS("background-color", transparent);
  await expect(session).toHaveCSS("box-shadow", noShadow);
  await expect(sessionShell).not.toHaveCSS("background-color", transparent);
  await expect(menu).toHaveCSS("opacity", "0");

  await projectShell.hover();
  await expect(projectShell).not.toHaveCSS("background-color", transparent);
  await expect(projectActions).toHaveCSS("opacity", "1");
  await projectMenu.hover();
  await expect(projectShell).not.toHaveCSS("background-color", transparent);
  await expect(projectMenu).toHaveCSS("background-color", transparent);
  await projectNewTask.hover();
  await expect(projectShell).not.toHaveCSS("background-color", transparent);
  await expect(projectNewTask).toHaveCSS("background-color", transparent);
  await expect(projectNewTask).toHaveCSS("box-shadow", noShadow);
  await sessionShell.hover();
  await expect(session).toHaveCSS("background-color", transparent);
  await expect(sessionShell).not.toHaveCSS("background-color", transparent);
  await expect(menu).toHaveCSS("opacity", "1");
  await menu.hover();
  await expect(sessionShell).not.toHaveCSS("background-color", transparent);
  await expect(menu).toHaveCSS("background-color", transparent);

  await page.mouse.move(0, 0);
  await expect(menu).toHaveCSS("opacity", "0");
  await menu.focus();
  await expect(menu).toHaveCSS("opacity", "1");
  await expect(session).toHaveAttribute("aria-current", "page");
  await menu.evaluate((element) => element.setAttribute("aria-expanded", "true"));
  await menu.blur();
  await expect(menu).toHaveCSS("opacity", "1");
});
