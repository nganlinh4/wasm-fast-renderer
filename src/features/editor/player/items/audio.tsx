import { IAudio } from "@designcombo/types";
import { BaseSequence, SequenceItemOptions } from "../base-sequence";
import { Audio as RemotionAudio } from "remotion";

export default function Audio({
	item,
	options,
}: {
	item: IAudio;
	options: SequenceItemOptions;
}) {
	const { fps } = options;
	const { details } = item;
	const playbackRate = item.playbackRate ?? 1;
	const children = (
		<RemotionAudio
			startFrom={Math.round(((item.trim?.from ?? 0) / 1000) * fps)}
			endAt={Math.max(
				Math.round(((item.trim?.to ?? 0) / 1000) * fps),
				Math.round(1 / fps)
			)}
			playbackRate={playbackRate}
			src={details.src}
			volume={(details.volume ?? 0) / 100}
		/>
	);
	return BaseSequence({ item, options, children });
}
