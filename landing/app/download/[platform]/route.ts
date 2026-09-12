import { latestDownloadUrl } from "../../lib/downloads";

export async function GET(
  _request: Request,
  { params }: { params: Promise<{ platform: string }> },
) {
  const { platform } = await params;
  if (platform !== "macos" && platform !== "linux") {
    return new Response("Unknown download platform", { status: 404 });
  }
  return new Response(null, {
    status: 302,
    headers: {
      Location: await latestDownloadUrl(platform),
      "Cache-Control": "no-store",
    },
  });
}
