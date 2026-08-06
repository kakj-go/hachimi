const request = (method, extra = {}) => {
  const requestId = crypto.randomUUID();
  window.parent.postMessage(
    {
      source: "hachimi-plugin-ui",
      protocolVersion: 1,
      request: { method, request_id: requestId, ...extra },
    },
    "*",
  );
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("bridge_timeout")), 5000);
    const receive = (event) => {
      const response = event.data?.response;
      if (event.source !== window.parent || response?.request_id !== requestId) return;
      clearTimeout(timer);
      window.removeEventListener("message", receive);
      if (response.kind === "error") reject(new Error(response.code));
      else resolve(response);
    };
    window.addEventListener("message", receive);
  });
};

const status = document.querySelector("#status");
const fixture = document.querySelector("#fixture");
const ipc = document.querySelector("#ipc");
ipc.textContent = "Direct Tauri IPC denied by host";
ipc.dataset.safe = "true";

try {
  const context = await request("get_context");
  status.textContent = `${context.value.pluginId}:${context.value.contributionId} · ${context.value.theme}`;
  const asset = await request("resolve_asset_url", {
    asset_contribution_id: "dashboard-assets",
    relative_path: "fixture.json",
  });
  const response = await fetch(asset.value);
  fixture.textContent = JSON.stringify(await response.json(), null, 2);
  fixture.dataset.loaded = "true";
} catch (error) {
  status.textContent = `Bridge error: ${String(error)}`;
}
