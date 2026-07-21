const status = document.querySelector("#status");
const send = message => chrome.runtime.sendMessage(message).then(result => {
  status.textContent = result?.message ?? (result?.ok ? "Connected to Bridge." : "Bridge is not connected.");
});

chrome.runtime.sendMessage({ type: "status" }).then(result => {
  status.textContent = result?.attached ? `Attached to ${result.title}` : result?.connected ? "Connected. This tab is not attached." : "Open Bridge, then connect again.";
});
document.querySelector("#attach").addEventListener("click", () => void send({ type: "attach-active" }));
document.querySelector("#detach").addEventListener("click", () => void send({ type: "detach-active" }));
