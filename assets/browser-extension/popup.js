const nonce = document.querySelector("#nonce");
const status = document.querySelector("#status");
document.querySelector("#pair").addEventListener("click", async () => {
  status.textContent = "Pairing…";
  const response = await chrome.runtime.sendMessage({ kind: "pair", nonce: nonce.value.trim() });
  status.textContent = response?.ok
    ? "Paired. Keep Chrome running while the task is active."
    : (response?.error ?? "Pairing failed.");
});
chrome.runtime.sendMessage({ kind: "status" }).then((response) => {
  if (response?.paired) status.textContent = "Paired with the local Hachimi app.";
});
