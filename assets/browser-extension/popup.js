const status = document.querySelector("#status");
document.querySelector("#pair").addEventListener("click", async () => {
  status.textContent = "Authorization requested. Confirm in Hachimi.";
  await chrome.runtime.sendMessage({ kind: "request_authorization" });
});
chrome.runtime.sendMessage({ kind: "status" }).then((response) => {
  if (response?.paired) status.textContent = "Paired with the local Hachimi app.";
});
