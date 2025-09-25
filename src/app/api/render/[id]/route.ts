import { NextResponse } from "next/server";

const RENDER_PORT = process.env.RENDER_PORT || "6108";
const RENDER_BASE = process.env.RENDER_BASE || `http://127.0.0.1:${RENDER_PORT}`;

export async function GET(
	request: Request,
	{ params }: { params: Promise<{ id: string }> },
) {
	try {
		const { id } = await params;
		if (!id) {
			return NextResponse.json(
				{ message: "id parameter is required" },
				{ status: 400 },
			);
		}

		const response = await fetch(`${RENDER_BASE}/render/${id}`, { cache: "no-store" });
		const statusData = await response.json();

		if (!response.ok) {
			const error = new Error(
				statusData?.message || "Failed status render video",
			);
			(error as any).status = response.status;
			throw error;
		}

		// Adapt to existing frontend contract: { video: { status, progress, url } }
		return NextResponse.json({ video: { status: statusData.status, progress: statusData.progress, url: statusData.url } }, { status: 200 });
	} catch (error: any) {
		console.error(error);
		return NextResponse.json(
			{ message: "Internal server error" },
			{ status: 500 },
		);
	}
}
