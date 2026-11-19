export function describeAttributes(a: Record<string, number>) {
  const parts: string[] = [];
  if (a["5"] > 0) parts.push(`${a["5"]}-bit`);
  if (a["4"] > 0) parts.push(`${Number((a["4"] / 1000).toFixed(3))} kHz`);
  if (a["0"] > 0) parts.push(`${a["0"]} kbps`);
  if (a["2"] === 1) parts.push("VBR");
  else if (a["2"] === 0) parts.push("CBR");
  if (a["1"] !== undefined) {
    const seconds = a["1"];
    parts.push(
      `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`,
    );
  }
  return parts.join(" · ");
}
export default function FileAttributes({
  attributes,
}: {
  attributes: Record<string, number>;
}) {
  const text = describeAttributes(attributes);
  return text ? (
    <small
      className="file-attributes"
      title="File attributes reported by the sharing client"
    >
      {text}
    </small>
  ) : null;
}
