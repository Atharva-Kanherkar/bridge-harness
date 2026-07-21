const video = document.querySelector("#stream");
const canvas = document.querySelector("#frame");
let stream;
let timer;

async function start(streamId) {
  stop();
  stream = await navigator.mediaDevices.getUserMedia({
    audio: false,
    video: { mandatory: { chromeMediaSource: "tab", chromeMediaSourceId: streamId } }
  });
  video.srcObject = stream;
  await video.play();
  stream.getVideoTracks()[0]?.addEventListener("ended", stop, { once: true });
  capture();
}

function capture() {
  clearTimeout(timer);
  if (!stream?.active) return;
  const width = video.videoWidth; const height = video.videoHeight;
  if (width && height) {
    const maxWidth = 1280; const scale = Math.min(1, maxWidth / width);
    canvas.width = Math.round(width * scale); canvas.height = Math.round(height * scale);
    canvas.getContext("2d").drawImage(video, 0, 0, canvas.width, canvas.height);
    chrome.runtime.sendMessage({ type: "bridge-live-frame", dataUrl: canvas.toDataURL("image/jpeg", 0.58), sourceWidth: width });
  }
  timer = setTimeout(capture, 700);
}

function stop() {
  clearTimeout(timer); timer = undefined;
  stream?.getTracks().forEach(track => track.stop()); stream = undefined; video.srcObject = null;
}

chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  if (message.type === "bridge-start-capture") { start(message.streamId).then(() => reply({ ok: true })).catch(error => reply({ ok: false, error: error.message })); return true; }
  if (message.type === "bridge-stop-capture") { stop(); reply({ ok: true }); }
});
