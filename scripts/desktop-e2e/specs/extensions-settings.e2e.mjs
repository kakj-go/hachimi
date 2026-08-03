import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { clickWhenReady, hoverWhenReady, waitForDisplayed } from "../support/interactions.mjs";
import { switchToWorkbench } from "../support/windows.mjs";

/* global HTMLElement, document, getComputedStyle */

async function clickDialogPrimary() {
  await clickWhenReady('[role="dialog"] .dialog-actions button:last-child');
}

async function openSkillActions(skillName) {
  const rowSelector = `[data-testid="skill-row-${skillName}"]`;
  const triggerSelector = `[data-testid="skill-actions-${skillName}"]`;
  await hoverWhenReady(rowSelector);
  await browser.waitUntil(
    async () =>
      browser.execute(
        (rowTarget, triggerTarget) => {
          const row = document.querySelector(rowTarget);
          const trigger = document.querySelector(triggerTarget);
          if (!(row instanceof HTMLElement) || !(trigger instanceof HTMLElement)) return false;
          row.focus();
          trigger.focus();
          const style = getComputedStyle(trigger);
          const bounds = trigger.getBoundingClientRect();
          return (
            document.activeElement === trigger &&
            style.display !== "none" &&
            style.visibility !== "hidden" &&
            bounds.width > 0 &&
            bounds.height > 0
          );
        },
        rowSelector,
        triggerSelector,
      ),
    { timeout: 20_000, timeoutMsg: `Skill actions did not become visible: ${skillName}` },
  );
  await clickWhenReady(triggerSelector);
}

async function selectMenuAction(selector) {
  await waitForDisplayed(selector);
  await clickWhenReady(selector);
}

async function openSettingsTab(tab) {
  await switchToWorkbench();
  if (!(await $(".settings-nav").isDisplayed())) {
    await clickWhenReady('[data-testid="workbench-open-settings"]');
    await waitForDisplayed(".settings-nav");
  }
  await clickWhenReady(`[data-testid="settings-nav-${tab}"]`);
}

async function replaceRichText(selector, value) {
  await waitForDisplayed(selector, 10_000);
  await browser.execute(
    (targetSelector, nextValue) => {
      const target = document.querySelector(targetSelector);
      if (!(target instanceof HTMLElement)) throw new Error("Rich text editor is unavailable");
      target.focus();
      target.replaceChildren(target.ownerDocument.createTextNode(nextValue));
      target.dispatchEvent(
        new target.ownerDocument.defaultView.InputEvent("input", {
          bubbles: true,
          data: nextValue,
          inputType: "insertText",
        }),
      );
    },
    selector,
    value,
  );
}

async function waitForEditorPath(path) {
  await browser.waitUntil(async () => (await $(".skill-editor-header strong").getText()) === path, {
    timeout: 20_000,
    timeoutMsg: `Skill editor did not switch to ${path}`,
  });
}

describe("Hachimi Skills and MCP settings", () => {
  before(async () => {
    await switchToWorkbench();
    await clickWhenReady('[data-testid="workbench-open-settings"]');
    await waitForDisplayed(".settings-nav");
  });

  it("creates, edits, watches, resolves conflicts and restores a Skill", async () => {
    await clickWhenReady('[data-testid="settings-nav-skills"]');
    await waitForDisplayed('[data-testid="skills-settings-page"]');
    await clickWhenReady('[data-testid="skill-create"]');
    await $('[placeholder*="release-notes"]').setValue("desktop-e2e-skill");
    await clickDialogPrimary();
    await waitForDisplayed('[data-testid="skill-row-desktop-e2e-skill"]');
    await openSkillActions("desktop-e2e-skill");
    await selectMenuAction('[data-testid="skill-action-new-file-desktop-e2e-skill"]');
    await waitForDisplayed('[data-testid="skill-entry-name"]');
    await $('[data-testid="skill-entry-name"]').setValue("reference.md");
    await clickDialogPrimary();
    await waitForEditorPath("reference.md");
    await replaceRichText(
      '[data-testid="skill-markdown-editor"]',
      "Desktop E2E reference loaded through SkillHost.\n",
    );
    await clickWhenReady('[data-testid="skill-save"]');

    await clickWhenReady('[data-testid="skill-node-SKILL.md"]');
    await waitForEditorPath("SKILL.md");
    await replaceRichText(
      '[data-testid="skill-markdown-editor"]',
      "# Desktop E2E Skill\n\n[Reference](reference.md)\n",
    );
    await clickWhenReady('[data-testid="skill-save"]');
    await waitForDisplayed(".skill-markdown-resource", 10_000);
    await clickWhenReady(".skill-markdown-resource summary");
    await browser.waitUntil(
      async () =>
        (await $(".skill-markdown-resource").getText()).includes(
          "Desktop E2E reference loaded through SkillHost",
        ),
      { timeout: 10_000, timeoutMsg: "Skill preview resource did not load through SkillHost" },
    );

    const localDraft = `${await browser.execute(
      () => document.querySelector('[data-testid="skill-markdown-editor"]')?.textContent ?? "",
    )}\nLocal draft wins after explicit confirmation.\n`;
    await replaceRichText('[data-testid="skill-markdown-editor"]', localDraft);
    const entryPath = join(
      process.env.HACHIMI_DATA_DIR,
      "skills",
      "user",
      "desktop-e2e-skill",
      "SKILL.md",
    );
    writeFileSync(entryPath, `${readFileSync(entryPath, "utf8")}\nExternal change.\n`, "utf8");
    await waitForDisplayed('[data-testid="skill-conflict-keep-local"]', 10_000);
    await clickWhenReady('[data-testid="skill-conflict-keep-local"]');
    await clickWhenReady('[data-testid="skill-save"]');

    await browser.refresh();
    await openSettingsTab("skills");
    await waitForDisplayed('[data-testid="skill-row-desktop-e2e-skill"]');
    await clickWhenReady('[data-testid="skill-node-SKILL.md"]');
    await browser.waitUntil(
      async () =>
        browser.execute(
          () =>
            document
              .querySelector('[data-testid="skill-markdown-editor"]')
              ?.textContent?.includes("Local draft wins") ?? false,
        ),
      { timeout: 10_000, timeoutMsg: "Local Skill draft was not restored" },
    );
  });

  it("tests, saves, enables, discovers and persistently disables an MCP Tool", async () => {
    await clickWhenReady('[data-testid="settings-nav-mcp"]');
    await waitForDisplayed('[data-testid="mcp-settings-page"]');
    await clickWhenReady('[data-testid="mcp-add-server"]');
    await $('[placeholder*="Filesystem"]').setValue("Desktop E2E MCP");
    await $('[placeholder="https://example.com/mcp"]').setValue(
      process.env.HACHIMI_DESKTOP_E2E_MCP_URL,
    );
    await clickWhenReady('[data-testid="mcp-test-new-connection"]');
    await browser.waitUntil(
      async () =>
        browser.execute(
          () =>
            document
              .querySelector('.mcp-create-dialog-form [data-component="status-banner"]')
              ?.textContent?.includes("连接成功") ?? false,
        ),
      { timeout: 20_000, timeoutMsg: "MCP connection test did not succeed" },
    );
    await waitForDisplayed('[data-testid="mcp-tool-echo"]');
    await clickWhenReady('[data-testid="mcp-save-new-server"]');

    await clickWhenReady('.mcp-detail-header [data-component="switch-root"]');
    await waitForDisplayed('[data-testid="mcp-tool-echo"]');
    await clickWhenReady('[data-testid="mcp-tool-echo"] [data-component="switch-root"]');
    await browser.waitUntil(
      async () =>
        !(await $(
          '[data-testid="mcp-tool-echo"] [data-component="switch-root"] input[type="checkbox"]',
        ).isSelected()),
      {
        timeout: 10_000,
        timeoutMsg: "MCP Tool toggle did not persist the disabled state",
      },
    );

    await browser.refresh();
    await openSettingsTab("mcp");
    await waitForDisplayed('[data-testid="mcp-tool-echo"]');
    await browser.waitUntil(
      async () =>
        browser.execute(
          () =>
            document.querySelector(
              '[data-testid="mcp-tool-echo"] [data-component="switch-root"] input[type="checkbox"]',
            )?.checked === false,
        ),
      { timeout: 10_000, timeoutMsg: "MCP Tool restored in the enabled state" },
    );
  });
});
