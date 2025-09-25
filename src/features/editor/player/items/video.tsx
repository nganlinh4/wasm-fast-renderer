import { IVideo } from "@designcombo/types";
import { BaseSequence, SequenceItemOptions } from "../base-sequence";
import { calculateMediaStyles } from "../styles";
import { OffthreadVideo } from "remotion";

export const Video = ({
	item,
	options,
}: {
	item: IVideo;
	options: SequenceItemOptions;
}) => {
	const { fps } = options;
	const { details, animations } = item;
	const playbackRate = item.playbackRate || 1;
	const crop = details?.crop || {
		x: 0,
		y: 0,
		width: details.width,
		height: details.height,
	};

	const children = (
		<div style={calculateMediaStyles(details, crop)}>
			<OffthreadVideo
				startFrom={Math.round(((item.trim?.from ?? 0) / 1000) * fps)}
				endAt={Math.max(
					Math.round(((item.trim?.to ?? 0) / 1000) * fps),
					Math.round(1 / fps)
				)}
				playbackRate={playbackRate ?? 1}
				src={details.src}
				volume={(details.volume ?? 0) / 100}
			/>
		</div>
	);

	return BaseSequence({ item, options, children });
};

export default Video;
