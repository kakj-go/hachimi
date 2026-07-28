import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { switchToWorkbench } from "../support/windows.mjs";

async function clickWhenReady(selector) {
  const element = await $(selector);
  await element.waitForDisplayed({ timeout: 20_000 });
  await element.waitForEnabled({ timeout: 20_000 });
  await element.click();
  return element;
}

async function clickDialogPrimary() {
  await clickWhenReady('[role="dialog"] .dialog-actions button:last-child');
}

async function openSettingsTab(tab) {
  await switchToWorkbench();
  if (!(await $(".settings-nav").isDisplayed())) {
    await clickWhenReady('[data-testid="workbench-open-settings"]');
    await $(".settings-nav").waitForDisplayed({ timeout: 20_000 });
  }
  await clickWhenReady(`[data-testid="settings-nav-${tab}"]`);
}

async function replaceRichText(element, value) {
  await element.waitForDisplayed({ timeout: 10_000 });
  await browser.execute(
    (target, nextValue) => {
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
    element,
    value,
  );
}

describe("Hachimi Skills and MCP settings", () => {
  before(async () => {
    await switchToWorkbench();
    await clickWhenReady('[data-testid="workbench-open-settings"]');
    await $(".settings-nav").waitForDisplayed({ timeout: 20_000 });
  });

  it("creates, edits, watches, resolves conflicts and restores a Skill", async () => {
    await clickWhenReady('[data-testid="settings-nav-skills"]');
    await $('[data-testid="skills-settings-page"]').waitForDisplayed({ timeout: 20_000 });
    await clickWhenReady('[data-testid="skill-create"]');
    await $('[placeholder*="release-notes"]').setValue("desktop-e2e-skill");
    await clickDialogPrimary();
    const skillRow = await $('[data-testid="skill-row-desktop-e2e-skill"]');
    await skillRow.waitForDisplayed({ timeout: 20_000 });
    await skillRow.moveTo();

    await clickWhenReady('[data-testid="skill-actions-desktop-e2e-skill"]');
    await clickWhenReady('[data-testid="skill-action-new-file-desktop-e2e-skill"]');
    await $('[data-testid="skill-entry-name"]').setValue("reference.md");
    await clickDialogPrimary();
    let editor = await $('[data-testid="skill-markdown-editor"]');
    await replaceRichText(editor, "Desktop E2E reference loaded through SkillHost.\n");
    await clickWhenReady('[data-testid="skill-save"]');

    await clickWhenReady('[data-testid="skill-node-SKILL.md"]');
    editor = await $('[data-testid="skill-markdown-editor"]');
    await replaceRichText(editor, "# Desktop E2E Skill\n\n[Reference](reference.md)\n");
    await clickWhenReady('[data-testid="skill-save"]');
    await $(".skill-markdown-resource").waitForDisplayed({ timeout: 10_000 });
    await clickWhenReady(".skill-markdown-resource summary");
    await expect($(".skill-markdown-resource")).toHaveText(
      expect.stringContaining("Desktop E2E reference loaded through SkillHost"),
    );

    const localDraft = `${await editor.getText()}\nLocal draft wins after explicit confirmation.\n`;
    await replaceRichText(editor, localDraft);
    const entryPath = join(
      process.env.HACHIMI_DATA_DIR,
      "skills",
      "user",
      "desktop-e2e-skill",
      "SKILL.md",
    );
    writeFileSync(entryPath, `${readFileSync(entryPath, "utf8")}\nExternal change.\n`, "utf8");
    await $('[data-testid="skill-conflict-keep-local"]').waitForDisplayed({ timeout: 10_000 });
    await clickWhenReady('[data-testid="skill-conflict-keep-local"]');
    await clickWhenReady('[data-testid="skill-save"]');

    await browser.refresh();
    await openSettingsTab("skills");
    await $('[data-testid="skill-row-desktop-e2e-skill"]').waitForDisplayed({ timeout: 20_000 });
    await clickWhenReady('[data-testid="skill-node-SKILL.md"]');
    await expect($('[data-testid="skill-markdown-editor"]')).toHaveText(
      expect.stringContaining("Local draft wins"),
    );
  });

  it("tests, saves, enables, discovers and persistently disables an MCP Tool", async () => {
    await clickWhenReady('[data-testid="settings-nav-mcp"]');
    await $('[data-testid="mcp-settings-page"]').waitForDisplayed({ timeout: 20_000 });
    await clickWhenReady('[data-testid="mcp-add-server"]');
    await $('[placeholder*="Filesystem"]').setValue("Desktop E2E MCP");
    await $('[placeholder="https://example.com/mcp"]').setValue(
      process.env.HACHIMI_DESKTOP_E2E_MCP_URL,
    );
    await clickWhenReady('[data-testid="mcp-test-new-connection"]');
    const connectionResult = await $('.mcp-create-dialog-form [data-component="status-banner"]');
    await connectionResult.waitForDisplayed({ timeout: 20_000 });
    await expect(connectionResult).toHaveText(expect.stringContaining("连接成功"));
    await $('[data-testid="mcp-tool-echo"]').waitForDisplayed({ timeout: 20_000 });
    await clickWhenReady('[data-testid="mcp-save-new-server"]');

    const serverToggle = await $('.mcp-detail-header [data-component="switch-root"]');
    await serverToggle.waitForDisplayed({ timeout: 10_000 });
    await serverToggle.click();
    await $('[data-testid="mcp-tool-echo"]').waitForDisplayed({ timeout: 20_000 });
    const toolToggle = await $('[data-testid="mcp-tool-echo"] [data-component="switch-root"]');
    await toolToggle.click();
    const toolInput = await toolToggle.$('input[type="checkbox"]');
    await browser.waitUntil(async () => !(await toolInput.isSelected()), {
      timeout: 10_000,
      timeoutMsg: "MCP Tool toggle did not persist the disabled state",
    });

    await browser.refresh();
    await openSettingsTab("mcp");
    await $('[data-testid="mcp-tool-echo"]').waitForDisplayed({ timeout: 20_000 });
    const restoredInput = await $(
      '[data-testid="mcp-tool-echo"] [data-component="switch-root"] input[type="checkbox"]',
    );
    await expect(restoredInput).not.toBeSelected();
  });
});
