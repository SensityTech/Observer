/** A message of the owner's as a player reads it, line breaks kept, under what it is. */
export function MessagePreview({ caption, message }: { caption: string; message: string }) {
    return (
        <figure className="m-0 flex flex-col gap-1.5 rounded-box border border-line bg-bg-subtle px-3.5 py-3">
            <figcaption className="text-meta text-ink-3">{caption}</figcaption>
            <p className="text-ui whitespace-pre-line text-ink [overflow-wrap:anywhere]">{message}</p>
        </figure>
    );
}
