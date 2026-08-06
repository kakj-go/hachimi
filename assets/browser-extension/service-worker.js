const endpoint = "http://127.0.0.1:42372";
let polling = false;
const networkResourceTypes = [
  "main_frame",
  "sub_frame",
  "stylesheet",
  "script",
  "image",
  "font",
  "object",
  "xmlhttprequest",
  "ping",
  "csp_report",
  "media",
  "websocket",
  "webtransport",
  "other",
];
const subresourceTypes = networkResourceTypes.filter((kind) => kind !== "main_frame");

async function storageGet() {
  return chrome.storage.local.get({ token: null, installId: null, sessions: {} });
}

async function extensionIdentity() {
  const state = await storageGet();
  let installId = state.installId;
  if (!installId) {
    installId = crypto.randomUUID();
    await chrome.storage.local.set({ installId });
  }
  return `${chrome.runtime.id}:${installId}`;
}

async function requestAuthorization() {
  const response = await fetch(`${endpoint}/v1/pair/request`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ extensionIdentity: await extensionIdentity() }),
  });
  if (response.status === 202) return false;
  if (!response.ok) return false;
  const body = await response.json();
  if (!body.token) return false;
  await chrome.storage.local.set({ token: body.token });
  return true;
}

async function api(path, token, body = {}) {
  return fetch(`${endpoint}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
    body: JSON.stringify(body),
  });
}

async function poll() {
  if (polling) return;
  polling = true;
  try {
    let { token } = await storageGet();
    if (!token) {
      if (!(await requestAuthorization())) return;
      ({ token } = await storageGet());
    }
    for (let count = 0; count < 8; count += 1) {
      const response = await api("/v1/commands/claim", token);
      if (response.status === 401) {
        const current = await sessions();
        for (const record of Object.values(current)) await clearNetworkRules(record);
        await chrome.storage.local.set({ sessions: {} });
        await chrome.storage.local.remove("token");
        return;
      }
      if (response.status === 204) break;
      if (!response.ok) throw new Error(`claim_failed_${response.status}`);
      const command = await response.json();
      const result = await execute(command).catch((error) => ({
        commandId: command.commandId,
        ok: false,
        errorCode: String(error?.message ?? error),
        observation: null,
        action: null,
      }));
      await api("/v1/commands/complete", token, result);
    }
  } catch {
    // The desktop app may be stopped. Commands remain queued locally by the Host.
  } finally {
    polling = false;
    setTimeout(() => void poll(), 750);
  }
}

export async function sessions() {
  return (await storageGet()).sessions;
}

async function setSession(sessionId, value) {
  const current = await sessions();
  if (value) current[sessionId] = value;
  else delete current[sessionId];
  await chrome.storage.local.set({ sessions: current });
}

async function clearNetworkRules(record) {
  const ruleIds = Array.isArray(record?.networkRuleIds) ? record.networkRuleIds : [];
  if (ruleIds.length) {
    await chrome.declarativeNetRequest.updateSessionRules({ removeRuleIds: ruleIds });
  }
}

function originRegex(origin) {
  const parsed = new URL(origin);
  if (!/^https?:$/.test(parsed.protocol) || parsed.origin !== origin) {
    throw new Error("extension_network_origin_invalid");
  }
  return `^${origin.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(?:/|$)`;
}

async function applyNetworkPolicy(sessionId, networkPolicy) {
  const { record, tabs } = await ownedTab(sessionId);
  await clearNetworkRules(record);
  const existing = await chrome.declarativeNetRequest.getSessionRules();
  let nextId = existing.reduce((largest, rule) => Math.max(largest, rule.id), 1000) + 1;
  const rules = [
    {
      id: nextId++,
      priority: 1,
      action: { type: "block" },
      condition: {
        tabIds: tabs.map((tab) => tab.id),
        regexFilter: "^https?://",
        resourceTypes: networkResourceTypes,
      },
    },
  ];
  const seen = new Set();
  for (const entry of networkPolicy?.rules ?? []) {
    if (entry.expiresAtMs != null && entry.expiresAtMs <= Date.now()) continue;
    if (entry.kind !== "document" && entry.kind !== "resource") {
      throw new Error("extension_network_rule_kind_invalid");
    }
    const ruleKey = `${entry.kind}:${entry.origin}`;
    if (seen.has(ruleKey)) continue;
    seen.add(ruleKey);
    rules.push({
      id: nextId++,
      priority: 2,
      action: { type: "allow" },
      condition: {
        tabIds: tabs.map((tab) => tab.id),
        regexFilter: originRegex(entry.origin),
        resourceTypes: entry.kind === "document" ? networkResourceTypes : subresourceTypes,
      },
    });
  }
  await chrome.declarativeNetRequest.updateSessionRules({ addRules: rules });
  await setSession(sessionId, {
    ...record,
    networkRuleIds: rules.map((rule) => rule.id),
    networkRevision: networkPolicy?.revision ?? 0,
    networkPolicy,
  });
}

async function ownedTab(sessionId) {
  const record = (await sessions())[sessionId];
  if (!record?.owned) throw new Error("extension_session_not_owned");
  const tab = await chrome.tabs.get(record.tabId).catch(() => null);
  if (!tab || tab.id !== record.tabId || tab.groupId !== record.groupId) {
    throw new Error("extension_owned_tab_missing");
  }
  const tabs = (await chrome.tabs.query({ groupId: record.groupId })).filter(
    (candidate) => Number.isInteger(candidate.id) && candidate.groupId === record.groupId,
  );
  if (!tabs.some((candidate) => candidate.id === tab.id)) {
    throw new Error("extension_owned_tab_missing");
  }
  return { record, tab, tabs };
}

export async function execute(command) {
  let observation = null;
  let action = null;
  if (command.kind === "start") {
    const tab = await chrome.tabs.create({
      url: "about:blank",
      active: true,
    });
    const groupId = await chrome.tabs.group({ tabIds: [tab.id] });
    await chrome.tabGroups.update(groupId, {
      title: command.taskTabGroup,
      color: "blue",
      collapsed: false,
    });
    await setSession(command.sessionId, {
      tabId: tab.id,
      groupId,
      owned: true,
      networkRuleIds: [],
    });
    await applyNetworkPolicy(command.sessionId, command.networkPolicy);
    if (command.initialUrl) {
      await chrome.tabs.update(tab.id, { url: command.initialUrl });
      await waitForTabNavigation(tab.id, "about:blank");
    }
  } else if (command.kind === "set_network_policy") {
    await applyNetworkPolicy(command.sessionId, command.networkPolicy);
  } else if (command.kind === "observe") {
    const { tab } = await ownedTab(command.sessionId);
    if (!/^https?:/.test(tab.url ?? "")) throw new Error("extension_forbidden_page_scheme");
    const injected = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => ({
        title: document.title,
        text: document.body?.innerText ?? "",
        url: location.href,
        viewportWidth: window.innerWidth,
        viewportHeight: window.innerHeight,
      }),
    });
    const screenshot = await withDebugger(tab.id, (send) =>
      send("Page.captureScreenshot", {
        format: "png",
        fromSurface: true,
        captureBeyondViewport: false,
      }),
    );
    observation = {
      ...(injected[0]?.result ?? { title: tab.title ?? "", text: "", url: tab.url }),
      screenshotBase64: screenshot.data,
    };
  } else if (command.kind === "act") {
    action = await executeAction(command.sessionId, command.expectedOrigin, command.action);
  } else if (command.kind === "resume") {
    const record = (await sessions())[command.sessionId];
    if (!record || record.owned) throw new Error("extension_session_not_released");
    const tab = await chrome.tabs.get(record.tabId).catch(() => null);
    if (!tab || !Number.isInteger(tab.id)) throw new Error("extension_released_tab_missing");
    const groupId = await chrome.tabs.group({ tabIds: [tab.id] });
    await chrome.tabGroups.update(groupId, {
      title: command.taskTabGroup,
      color: "blue",
      collapsed: false,
    });
    await setSession(command.sessionId, {
      ...record,
      groupId,
      owned: true,
      networkRuleIds: [],
    });
    try {
      await applyNetworkPolicy(command.sessionId, command.networkPolicy);
    } catch (error) {
      await chrome.tabs.ungroup([tab.id]).catch(() => {});
      await setSession(command.sessionId, {
        ...record,
        owned: false,
        networkRuleIds: [],
      });
      throw error;
    }
  } else if (command.kind === "take_over") {
    const { record, tabs } = await ownedTab(command.sessionId);
    await clearNetworkRules(record);
    await chrome.tabs.ungroup(tabs.map((tab) => tab.id)).catch(() => {});
    await setSession(command.sessionId, { ...record, owned: false, networkRuleIds: [] });
  } else if (command.kind === "stop") {
    const record = (await sessions())[command.sessionId];
    if (!record) throw new Error("extension_session_missing");
    await clearNetworkRules(record);
    if (record.owned) {
      const { tabs } = await ownedTab(command.sessionId);
      await setSession(command.sessionId, null);
      await chrome.tabs.remove(tabs.map((tab) => tab.id));
    } else {
      await setSession(command.sessionId, null);
    }
  } else {
    throw new Error("extension_command_unsupported");
  }
  const current = (await sessions())[command.sessionId];
  return {
    commandId: command.commandId,
    ok: true,
    errorCode: null,
    observation,
    action,
    ownerTabId: current?.owned && Number.isInteger(current.tabId) ? current.tabId : null,
  };
}

async function withDebugger(tabId, operation) {
  const target = { tabId };
  await chrome.debugger.attach(target, "1.3");
  try {
    return await operation((method, params = {}) =>
      chrome.debugger.sendCommand(target, method, params),
    );
  } finally {
    await chrome.debugger.detach(target).catch(() => {});
  }
}

function assertCdpShape(method, params) {
  if (!params || Array.isArray(params) || typeof params !== "object") {
    throw new Error("extension_cdp_params_invalid");
  }
  const exactKeys = (allowed) => {
    if (Object.keys(params).some((key) => !allowed.includes(key))) {
      throw new Error("extension_cdp_params_invalid");
    }
  };
  if (method === "DOM.getDocument") {
    exactKeys(["depth", "pierce"]);
    if (
      (params.depth != null &&
        (!Number.isInteger(params.depth) || params.depth < 0 || params.depth > 2)) ||
      params.pierce === true
    ) {
      throw new Error("extension_cdp_params_invalid");
    }
  } else if (method === "DOM.querySelector") {
    exactKeys(["nodeId", "selector"]);
    if (
      !Number.isInteger(params.nodeId) ||
      params.nodeId <= 0 ||
      typeof params.selector !== "string" ||
      !params.selector.trim() ||
      params.selector.length > 4096
    ) {
      throw new Error("extension_cdp_params_invalid");
    }
  } else if (method === "DOM.getAttributes" || method === "DOM.getBoxModel") {
    exactKeys(["nodeId"]);
    if (!Number.isInteger(params.nodeId) || params.nodeId <= 0) {
      throw new Error("extension_cdp_params_invalid");
    }
  } else if (method === "Page.getLayoutMetrics" || method === "Page.stopLoading") {
    exactKeys([]);
  } else if (method === "Page.reload") {
    exactKeys(["ignoreCache"]);
    if (params.ignoreCache != null && typeof params.ignoreCache !== "boolean") {
      throw new Error("extension_cdp_params_invalid");
    }
  } else {
    throw new Error("extension_cdp_method_unsupported");
  }
}

async function executeCdp(tabId, method, params) {
  assertCdpShape(method, params);
  return withDebugger(tabId, (send) => send(method, params));
}

async function dispatchBrowserKeys(tabId, keys) {
  const modifiers = new Map([
    ["alt", 1],
    ["ctrl", 2],
    ["control", 2],
    ["meta", 4],
    ["command", 4],
    ["shift", 8],
  ]);
  const normalized = keys.map((key) => String(key));
  const isChord =
    normalized.length > 1 &&
    normalized.slice(0, -1).every((key) => modifiers.has(key.toLowerCase()));
  const presses = isChord ? [normalized.at(-1)] : normalized;
  const modifierMask = isChord
    ? normalized.slice(0, -1).reduce((mask, key) => mask | modifiers.get(key.toLowerCase()), 0)
    : 0;
  await withDebugger(tabId, async (send) => {
    for (const key of presses) {
      const printable = key.length === 1 && (modifierMask & 7) === 0 ? key : undefined;
      await send("Input.dispatchKeyEvent", {
        type: "keyDown",
        key,
        text: printable,
        modifiers: modifierMask,
      });
      await send("Input.dispatchKeyEvent", { type: "keyUp", key, modifiers: modifierMask });
    }
  });
}

async function waitForBrowserState(tabId, selector, state, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() <= deadline) {
    if (state === "navigation_complete") {
      const tab = await chrome.tabs.get(tabId);
      if (tab.status === "complete") return;
    } else {
      const injected = await chrome.scripting.executeScript({
        target: { tabId },
        args: [selector, state],
        func: (candidate, expected) => {
          const element = document.querySelector(candidate);
          if (expected === "attached") return Boolean(element);
          if (expected === "hidden") {
            if (!element) return true;
            const style = getComputedStyle(element);
            const box = element.getBoundingClientRect();
            return (
              style.display === "none" || style.visibility === "hidden" || !box.width || !box.height
            );
          }
          if (!element) return false;
          const style = getComputedStyle(element);
          const box = element.getBoundingClientRect();
          return (
            style.display !== "none" &&
            style.visibility !== "hidden" &&
            box.width > 0 &&
            box.height > 0
          );
        },
      });
      if (injected[0]?.result === true) return;
    }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error("extension_browser_wait_timeout");
}

async function waitForTabNavigation(tabId, previousUrl) {
  const deadline = Date.now() + 30_000;
  while (Date.now() <= deadline) {
    const candidate = await chrome.tabs.get(tabId);
    if (candidate.url !== previousUrl && candidate.status === "complete") return candidate;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error("extension_navigation_timeout");
}

async function executeAction(sessionId, expectedOrigin, request) {
  let { record, tab, tabs } = await ownedTab(sessionId);
  if (!/^https?:/.test(tab.url ?? "") || new URL(tab.url).origin !== expectedOrigin) {
    throw new Error("extension_stale_origin");
  }
  let resultCode = "accepted";
  let output = null;
  if (request.kind === "navigate") {
    await chrome.tabs.update(tab.id, { url: request.url });
    resultCode = "navigated";
  } else if (request.kind === "back") {
    await chrome.tabs.goBack(tab.id);
    const current = await waitForTabNavigation(tab.id, tab.url);
    output = { tabId: String(tab.id), origin: new URL(current.url).origin };
    resultCode = "history_back";
  } else if (request.kind === "forward") {
    await chrome.tabs.goForward(tab.id);
    const current = await waitForTabNavigation(tab.id, tab.url);
    output = { tabId: String(tab.id), origin: new URL(current.url).origin };
    resultCode = "history_forward";
  } else if (request.kind === "reload") {
    await chrome.tabs.reload(tab.id, { bypassCache: Boolean(request.ignore_cache) });
    resultCode = "reloaded";
  } else if (request.kind === "stop") {
    await executeCdp(tab.id, "Page.stopLoading", {});
    resultCode = "loading_stopped";
  } else if (
    [
      "click",
      "hover",
      "double_click",
      "scroll",
      "drag_drop",
      "clear",
      "fill",
      "select_option",
      "type_text",
    ].includes(request.kind)
  ) {
    await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      args: [request],
      func: (action) => {
        const find = (selector) => {
          const element = document.querySelector(selector);
          if (!element) throw new Error("selector_not_found");
          return element;
        };
        const emitValue = (element, value, append) => {
          element.focus();
          element.value = append ? `${element.value ?? ""}${value}` : value;
          element.dispatchEvent(
            new InputEvent("input", { bubbles: true, inputType: "insertText", data: value }),
          );
          element.dispatchEvent(new Event("change", { bubbles: true }));
        };
        if (action.kind === "click") find(action.selector).click();
        else if (action.kind === "hover") {
          const element = find(action.selector);
          for (const kind of ["mouseover", "mouseenter", "mousemove"]) {
            element.dispatchEvent(
              new MouseEvent(kind, { bubbles: true, cancelable: true, view: window }),
            );
          }
        } else if (action.kind === "double_click") {
          const element = find(action.selector);
          element.click();
          element.click();
          element.dispatchEvent(
            new MouseEvent("dblclick", {
              bubbles: true,
              cancelable: true,
              detail: 2,
              view: window,
            }),
          );
        } else if (action.kind === "scroll") {
          const target = action.selector ? find(action.selector) : window;
          target.scrollBy({ left: action.delta_x, top: action.delta_y, behavior: "instant" });
        } else if (action.kind === "drag_drop") {
          const source = find(action.source_selector);
          const target = find(action.target_selector);
          const data = new DataTransfer();
          source.dispatchEvent(
            new DragEvent("dragstart", { bubbles: true, cancelable: true, dataTransfer: data }),
          );
          target.dispatchEvent(
            new DragEvent("dragenter", { bubbles: true, cancelable: true, dataTransfer: data }),
          );
          target.dispatchEvent(
            new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }),
          );
          target.dispatchEvent(
            new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }),
          );
          source.dispatchEvent(
            new DragEvent("dragend", { bubbles: true, cancelable: true, dataTransfer: data }),
          );
        } else if (action.kind === "clear") emitValue(find(action.selector), "", false);
        else if (action.kind === "fill") emitValue(find(action.selector), action.text, false);
        else if (action.kind === "type_text") emitValue(find(action.selector), action.text, true);
        else if (action.kind === "select_option") {
          const element = find(action.selector);
          if (!(element instanceof HTMLSelectElement)) throw new Error("selector_not_select");
          element.value = action.value;
          if (element.value !== action.value) throw new Error("select_option_missing");
          element.dispatchEvent(new Event("input", { bubbles: true }));
          element.dispatchEvent(new Event("change", { bubbles: true }));
        }
      },
    });
    resultCode = {
      click: "clicked",
      hover: "hovered",
      double_click: "double_clicked",
      scroll: "scrolled",
      drag_drop: "dragged",
      clear: "cleared",
      fill: "filled",
      select_option: "option_selected",
      type_text: "typed",
    }[request.kind];
  } else if (request.kind === "press_keys") {
    await dispatchBrowserKeys(tab.id, request.keys);
    resultCode = "keys_pressed";
  } else if (request.kind === "wait_for") {
    await waitForBrowserState(tab.id, request.selector, request.state, request.timeout_ms);
    resultCode = "wait_satisfied";
  } else if (request.kind === "tab_list") {
    output = tabs.map((candidate) => ({
      tabId: String(candidate.id),
      url: candidate.url ?? "",
      title: candidate.title ?? "",
      active: candidate.id === tab.id,
    }));
    resultCode = "tabs_listed";
  } else if (request.kind === "tab_new") {
    const candidate = await chrome.tabs.create({ url: "about:blank", active: true });
    await chrome.tabs.group({ groupId: record.groupId, tabIds: [candidate.id] });
    record = { ...record, tabId: candidate.id };
    await setSession(sessionId, record);
    await applyNetworkPolicy(sessionId, record.networkPolicy);
    if (request.url) await chrome.tabs.update(candidate.id, { url: request.url });
    tab = await chrome.tabs.get(candidate.id);
    output = {
      tabId: String(candidate.id),
      origin: request.url ? new URL(request.url).origin : null,
    };
    resultCode = "tab_created";
  } else if (request.kind === "tab_switch") {
    const candidate = tabs.find((owned) => String(owned.id) === request.tab_id);
    if (!candidate) throw new Error("extension_tab_not_owned");
    await chrome.tabs.update(candidate.id, { active: true });
    record = { ...record, tabId: candidate.id };
    await setSession(sessionId, record);
    tab = await chrome.tabs.get(candidate.id);
    output = { tabId: String(candidate.id), origin: new URL(tab.url).origin };
    resultCode = "tab_switched";
  } else if (request.kind === "tab_close") {
    if (tabs.length <= 1) throw new Error("extension_last_owned_tab");
    const closing = tabs.find((owned) => String(owned.id) === request.tab_id);
    if (!closing) throw new Error("extension_tab_not_owned");
    const replacement = closing.id === tab.id ? tabs.find((owned) => owned.id !== closing.id) : tab;
    record = { ...record, tabId: replacement.id };
    await setSession(sessionId, record);
    await chrome.tabs.remove(closing.id);
    await applyNetworkPolicy(sessionId, record.networkPolicy);
    tab = await chrome.tabs.get(replacement.id);
    output = { tabId: String(replacement.id), origin: new URL(tab.url).origin };
    resultCode = "tab_closed";
  } else if (request.kind === "upload") {
    const target = { tabId: tab.id };
    await chrome.debugger.attach(target, "1.3");
    try {
      const document = await chrome.debugger.sendCommand(target, "DOM.getDocument", { depth: 0 });
      const node = await chrome.debugger.sendCommand(target, "DOM.querySelector", {
        nodeId: document.root.nodeId,
        selector: request.selector,
      });
      if (!node.nodeId) throw new Error("selector_not_found");
      await chrome.debugger.sendCommand(target, "DOM.setFileInputFiles", {
        nodeId: node.nodeId,
        files: [request.file_token],
      });
      resultCode = "uploaded";
    } finally {
      await chrome.debugger.detach(target).catch(() => {});
    }
  } else if (request.kind === "download") {
    const startedAt = new Date().toISOString();
    const before = new Set((await chrome.downloads.search({ limit: 1000 })).map((item) => item.id));
    const target = { tabId: tab.id };
    await chrome.debugger.attach(target, "1.3");
    let ownedDownload;
    try {
      await chrome.debugger.sendCommand(target, "Page.enable");
      const downloadStarted = new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          chrome.debugger.onEvent.removeListener(onEvent);
          reject(new Error("extension_download_event_timeout"));
        }, 20_000);
        const onEvent = (source, method, params) => {
          if (source.tabId !== tab.id || method !== "Page.downloadWillBegin") return;
          clearTimeout(timer);
          chrome.debugger.onEvent.removeListener(onEvent);
          resolve(params);
        };
        chrome.debugger.onEvent.addListener(onEvent);
      });
      await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        args: [request.selector],
        func: (selector) => {
          const element = document.querySelector(selector);
          if (!element) throw new Error("selector_not_found");
          element.click();
        },
      });
      ownedDownload = await downloadStarted;
    } finally {
      await chrome.debugger.detach(target).catch(() => {});
    }
    let downloaded = null;
    for (let attempt = 0; attempt < 200; attempt += 1) {
      const items = await chrome.downloads.search({
        startedAfter: startedAt,
        orderBy: ["-startTime"],
        limit: 10,
      });
      const candidates = items.filter(
        (item) =>
          item.state === "complete" &&
          !item.error &&
          !before.has(item.id) &&
          (item.finalUrl === ownedDownload.url || item.url === ownedDownload.url),
      );
      if (candidates.length > 1) throw new Error("extension_download_ambiguous");
      downloaded = candidates[0] ?? null;
      if (downloaded) break;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    if (!downloaded?.filename) throw new Error("extension_download_timeout");
    resultCode = "download_quarantined";
    output = {
      hostDownloadPath: downloaded.filename,
      fileName: downloaded.filename.split(/[\\/]/).pop(),
      declaredMime: downloaded.mime || "application/octet-stream",
      downloadId: downloaded.id,
      downloadGuid: ownedDownload.guid,
      ownerTabId: tab.id,
    };
  } else if (request.kind === "read_storage") {
    const local = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => Object.fromEntries(Object.entries(localStorage)),
    });
    const cookies = await chrome.cookies.getAll({ url: tab.url });
    output = { localStorage: local[0]?.result ?? {}, cookies };
    resultCode = "storage_read";
  } else if (request.kind === "write_storage") {
    await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      args: [request.entries],
      func: (entries) =>
        Object.entries(entries).forEach(([key, value]) => localStorage.setItem(key, String(value))),
    });
    resultCode = "storage_written";
  } else if (request.kind === "cdp") {
    output = await executeCdp(tab.id, request.method, request.params);
    resultCode = "cdp_allowlisted";
  } else {
    throw new Error("extension_action_unsupported");
  }
  return { resultCode, output };
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.kind === "request_authorization") {
    requestAuthorization()
      .then((approved) => sendResponse({ ok: true, approved }))
      .catch((error) => sendResponse({ ok: false, error: String(error.message ?? error) }));
    return true;
  }
  if (message?.kind === "status") {
    storageGet().then((state) => sendResponse({ paired: Boolean(state.token) }));
    return true;
  }
  return false;
});

chrome.tabs.onRemoved.addListener(async (tabId) => {
  const current = await sessions();
  for (const [sessionId, record] of Object.entries(current)) {
    if (record.tabId === tabId) {
      const replacements = await chrome.tabs.query({ groupId: record.groupId });
      if (record.owned && replacements.length) {
        await setSession(sessionId, { ...record, tabId: replacements[0].id });
        await applyNetworkPolicy(sessionId, record.networkPolicy).catch(async () => {
          await clearNetworkRules(record);
          await setSession(sessionId, null);
        });
      } else {
        await clearNetworkRules(record);
        await setSession(sessionId, null);
      }
    }
  }
});

chrome.tabs.onReplaced.addListener(async (addedTabId, removedTabId) => {
  const current = await sessions();
  for (const [sessionId, record] of Object.entries(current)) {
    if (record.tabId === removedTabId) {
      await clearNetworkRules(record);
      await setSession(sessionId, {
        ...record,
        tabId: addedTabId,
        owned: false,
        networkRuleIds: [],
      });
    }
  }
});

chrome.runtime.onStartup.addListener(() => void poll());
chrome.runtime.onInstalled.addListener(() => void poll());
void poll();
