import { Fragment, type ReactNode } from "react";
function inline(value: string): ReactNode {
  return value
    .split(/(\*\*[^*]+\*\*|`[^`]+`)/g)
    .map((part, index) =>
      part.startsWith("**") && part.endsWith("**") ? (
        <strong key={index}>{part.slice(2, -2)}</strong>
      ) : part.startsWith("`") && part.endsWith("`") ? (
        <code key={index}>{part.slice(1, -1)}</code>
      ) : (
        <Fragment key={index}>{part}</Fragment>
      ),
    );
}
export function MessageText({ text }: { text: string }) {
  const blocks: ReactNode[] = [];
  let paragraph: string[] = [];
  let list: string[] = [];
  const flush = () => {
    if (paragraph.length) {
      blocks.push(<p key={blocks.length}>{inline(paragraph.join("\n"))}</p>);
      paragraph = [];
    }
    if (list.length) {
      blocks.push(
        <ul key={blocks.length}>
          {list.map((line, index) => (
            <li key={index}>{inline(line)}</li>
          ))}
        </ul>,
      );
      list = [];
    }
  };
  for (const line of text.split("\n")) {
    if (!line.trim()) {
      flush();
      continue;
    }
    if (/^#{1,4} /.test(line)) {
      flush();
      blocks.push(
        <h3 key={blocks.length}>{inline(line.replace(/^#+ /, ""))}</h3>,
      );
    } else if (/^[-*] /.test(line)) {
      if (paragraph.length) flush();
      list.push(line.slice(2));
    } else {
      if (list.length) flush();
      paragraph.push(line);
    }
  }
  flush();
  return <div className="message-markdown">{blocks}</div>;
}
