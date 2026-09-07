const video = document.querySelector("#stream");
const canvas = document.querySelector("#frame");
let stream;
let timer;
let encoding = false;
let redactions = [];
let viewportWidth = 1;
let sequence = 0;
let redactionEpoch = 0;

const FRAME_INTERVAL_MS = 100;
const MAX_FRAME_WIDTH = 1100;

async function start(streamId) {
  stop();
  stream = await navigator.mediaDevices.getUserMedia({
    audio: false,
    video: { mandatory: { chromeMediaSource: "tab", chromeMediaSourceId: streamId } }
  });
  video.srcObject = stream;
  await video.play();
  stream.getVideoTracks()[0]?.addEventListener("ended", stop, { once: true });
  scheduleCapture(0);
}

function scheduleCapture(delay = FRAME_INTERVAL_MS) {
  clearTimeout(timer);
  if (stream?.active) timer = setTimeout(() => void capture(), delay);
}

async function capture() {
  if (!stream?.active || encoding) { scheduleCapture(); return; }
  const width = video.videoWidth; const height = video.videoHeight;
  if (width && height) {
    encoding = true;
    const frameEpoch = redactionEpoch;
    try {
      const scale = Math.min(1, MAX_FRAME_WIDTH / width);
      const nextWidth = Math.round(width * scale); const nextHeight = Math.round(height * scale);
      if (canvas.width !== nextWidth) canvas.width = nextWidth;
      if (canvas.height !== nextHeight) canvas.height = nextHeight;
      const context = canvas.getContext("2d", { alpha: false });
      context.drawImage(video, 0, 0, canvas.width, canvas.height);
      const redactionScale = canvas.width / Math.max(1, viewportWidth);
      context.fillStyle = "#151517";
      for (const region of redactions) context.fillRect(region.x * redactionScale, region.y * redactionScale, region.width * redactionScale, region.height * redactionScale);
      const blob = await new Promise(resolve => canvas.toBlob(resolve, "image/webp", 0.52));
      if (!blob) throw new Error("Mirror frame encoding failed");
      const dataUrl = await new Promise((resolve, reject) => {
        const reader = new FileReader(); reader.onload = () => resolve(reader.result); reader.onerror = reject; reader.readAsDataURL(blob);
      });
      await chrome.runtime.sendMessage({ type: "bridge-live-frame", dataUrl, redactedRegions: redactions.length, sequence: ++sequence, redactionEpoch: frameEpoch });
    } catch (error) {
      chrome.runtime.sendMessage({ type: "bridge-capture-error", error: error.message }).catch(() => {});
    } finally {
      encoding = false;
      scheduleCapture();
    }
    return;
  }
  scheduleCapture();
}

function stop() {
  clearTimeout(timer); timer = undefined; encoding = false;
  stream?.getTracks().forEach(track => track.stop()); stream = undefined; video.srcObject = null;
}

chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  if (message.type === "bridge-start-capture") { start(message.streamId).then(() => reply({ ok: true })).catch(error => reply({ ok: false, error: error.message })); return true; }
  if (message.type === "bridge-update-redactions") {
    redactions = Array.isArray(message.regions) ? message.regions : [];
    viewportWidth = Math.max(1, Number(message.viewportWidth) || 1);
    redactionEpoch = Number(message.redactionEpoch) || 0;
    reply({ ok: true }); return;
  }
  if (message.type === "bridge-stop-capture") { stop(); reply({ ok: true }); }
});
