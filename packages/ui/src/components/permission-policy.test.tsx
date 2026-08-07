import { render } from "solid-js/web";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { PermissionPolicyEditor, type PermissionPolicyValue } from "./permission-policy";

function policy(): PermissionPolicyValue {
  return {
    level: "writable",
    revision: 0,
    rules: {
      fileSystem: [],
      network: { enabled: false, hosts: [], protocols: [] },
      process: { spawn: false, interactive: false, allowedCommands: [] },
      browser: {
        observe: false,
        act: false,
        upload: false,
        download: false,
        cookieStorage: false,
        cdp: false,
        origins: [],
      },
      computer: { observe: false, act: false, allowedApplications: [], maxActions: null },
      mcp: [],
      connectors: [],
    },
  };
}

describe("PermissionPolicyEditor", () => {
  it("adds a recursive directory and keeps fine rules separate", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const value = policy();
    const onChange = vi.fn();
    const dispose = render(
      () => (
        <PermissionPolicyEditor
          value={value}
          zh
          onChange={onChange}
          chooseDirectory={vi.fn().mockResolvedValue("C:\\workspace\\shared")}
        />
      ),
      host,
    );
    const directoryButton = host.querySelector<HTMLButtonElement>(
      '[data-component="permission-policy-grid"] section:first-child .permission-policy-list button',
    );
    expect(directoryButton).toBeTruthy();
    await userEvent.click(directoryButton!);
    await Promise.resolve();
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        rules: expect.objectContaining({
          fileSystem: [
            expect.objectContaining({
              access: "read",
              roots: ["C:\\workspace\\shared"],
              files: [],
            }),
          ],
        }),
      }),
    );
    dispose();
    host.remove();
  });

  it("uses human browser labels and adds validated hosts and origins", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const value = policy();
    const onChange = vi.fn();
    const dispose = render(
      () => <PermissionPolicyEditor value={value} zh onChange={onChange} />,
      host,
    );
    expect(host).toHaveTextContent("查看网页内容");
    expect(host).toHaveTextContent("高级浏览器控制");
    const inputs = [...host.querySelectorAll<HTMLInputElement>("input")];
    const hostInput = inputs.find((input) => input.placeholder.includes("域名"))!;
    await userEvent.type(hostInput, "example.com{Enter}");
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        rules: expect.objectContaining({
          network: expect.objectContaining({ hosts: ["example.com"], enabled: true }),
        }),
      }),
    );
    const originInput = inputs.find((input) => input.placeholder.includes("Origin"))!;
    await userEvent.type(originInput, "https://example.com{Enter}");
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        rules: expect.objectContaining({
          browser: expect.objectContaining({ origins: ["https://example.com"] }),
        }),
      }),
    );
    dispose();
    host.remove();
  });

  it("changes unrestricted scope without changing its capability or prompting immediately", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const value = policy();
    const onChange = vi.fn();
    const confirm = vi.spyOn(window, "confirm");
    const dispose = render(
      () => <PermissionPolicyEditor value={value} zh onChange={onChange} />,
      host,
    );
    const allApplications = [...host.querySelectorAll("label")].find((label) =>
      label.textContent?.includes("所有应用"),
    );
    expect(allApplications).toBeTruthy();
    await userEvent.click(allApplications!);
    expect(confirm).not.toHaveBeenCalled();
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        rules: expect.objectContaining({
          computer: expect.objectContaining({ unrestrictedTargets: true, observe: false }),
        }),
      }),
    );
    confirm.mockRestore();
    dispose();
    host.remove();
  });

  it("searches commands with Tab and adds the resolved executable path", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const onChange = vi.fn();
    const searchCommands = vi
      .fn()
      .mockResolvedValue([{ name: "git", executablePath: "C:\\Tools\\git.exe", source: "PATH" }]);
    const dispose = render(
      () => (
        <PermissionPolicyEditor
          value={policy()}
          zh
          onChange={onChange}
          searchCommands={searchCommands}
        />
      ),
      host,
    );
    const commandList = [...host.querySelectorAll(".permission-policy-list")].find((item) =>
      item.textContent?.includes("允许的命令"),
    );
    await userEvent.click(commandList!.querySelector("button")!);
    const input = document.querySelector<HTMLInputElement>('input[placeholder*="命令前缀"]')!;
    await userEvent.type(input, "git");
    await userEvent.keyboard("{Tab}");
    await vi.waitFor(() => expect(searchCommands).toHaveBeenCalledWith("git"));
    const result = [...document.querySelectorAll("label")].find((label) =>
      label.textContent?.includes("C:\\Tools\\git.exe"),
    );
    await userEvent.click(result!);
    const add = [...document.querySelectorAll("button")].find((button) =>
      button.textContent?.includes("添加选中命令"),
    );
    await userEvent.click(add!);
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        rules: expect.objectContaining({
          process: expect.objectContaining({ allowedCommands: ["C:\\Tools\\git.exe"] }),
        }),
      }),
    );
    dispose();
    host.remove();
  });

  it("shows invalid Origin feedback and refreshes application candidates", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const listApplications = vi.fn().mockResolvedValue([]);
    const dispose = render(
      () => (
        <PermissionPolicyEditor
          value={policy()}
          zh
          onChange={vi.fn()}
          listApplications={listApplications}
        />
      ),
      host,
    );
    const origin = host.querySelector<HTMLInputElement>('input[placeholder*="Origin"]')!;
    await userEvent.type(origin, "https://example.com/path{Enter}");
    expect(host).toHaveTextContent("请输入仅包含协议、主机和可选端口的 Origin");
    const appList = [...host.querySelectorAll(".permission-policy-list")].find((item) =>
      item.textContent?.includes("允许的应用"),
    );
    await userEvent.click(appList!.querySelector("button")!);
    await vi.waitFor(() => expect(listApplications).toHaveBeenCalledTimes(1));
    const refresh = [...document.querySelectorAll("button")].find(
      (button) => button.textContent?.trim() === "刷新",
    );
    await userEvent.click(refresh!);
    await vi.waitFor(() => expect(listApplications).toHaveBeenCalledTimes(2));
    dispose();
    host.remove();
  });
});
