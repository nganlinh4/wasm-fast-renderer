import { NextResponse } from "next/server";

const RENDER_PORT = process.env.RENDER_PORT || "6108";
const RENDER_BASE = process.env.RENDER_BASE || `http://127.0.0.1:${RENDER_PORT}`;

export async function GET(
  _request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    if (!id) {
      return NextResponse.json(
        { message: "id parameter is required" },
        { status: 400 }
      );
    }

    const upstream = await fetch(`${RENDER_BASE}/render/${id}/output`, {
      cache: "no-store",
    });
    if (!upstream.ok || !upstream.body) {
      return NextResponse.json(
        { message: "Failed to fetch rendered output" },
        { status: upstream.status || 502 }
      );
    }

    // Stream bytes back to the browser under same-origin
    return new Response(upstream.body, {
      status: 200,
      headers: {
        "Content-Type": "video/mp4",
        "Cache-Control": "no-store",
      },
    });
  } catch (error) {
    console.error(error);
    return NextResponse.json(
      { message: "Internal server error" },
      { status: 500 }
    );
  }
}

