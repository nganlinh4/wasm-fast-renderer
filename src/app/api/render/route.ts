import { NextResponse } from "next/server";

const RENDER_PORT = process.env.RENDER_PORT || "6108";
const RENDER_BASE = process.env.RENDER_BASE || `http://127.0.0.1:${RENDER_PORT}`;

export async function POST(request: Request) {
	try {
		const body = await request.json();
		const response = await fetch(`${RENDER_BASE}/render`, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
		});
		const responseData = await response.json();
		if (!response.ok) {
			return NextResponse.json(
				{ message: responseData?.message || "Failed to start local render" },
				{ status: response.status },
			);
		}
		// Adapt to existing frontend contract: { video: { id } }
		return NextResponse.json({ video: { id: responseData.jobId } }, { status: 200 });
	} catch (error) {
		console.error(error);
		return NextResponse.json(
			{ message: "Internal server error" },
			{ status: 500 },
		);
	}
}

export async function GET(request: Request) {
	try {
		const { searchParams } = new URL(request.url);
		const id = searchParams.get("id");
		if (!id) {
			return NextResponse.json(
				{ message: "id parameter is required" },
				{ status: 400 },
			);
		}
		const response = await fetch(`${RENDER_BASE}/render/${id}`, { cache: "no-store" });
		if (!response.ok) {
			return NextResponse.json(
				{ message: "Failed to fetch local render status" },
				{ status: response.status },
			);
		}
		const statusData = await response.json();
		return NextResponse.json(statusData, { status: 200 });
	} catch (error) {
		console.error(error);
		return NextResponse.json(
			{ message: "Internal server error" },
			{ status: 500 },
		);
	}
}
