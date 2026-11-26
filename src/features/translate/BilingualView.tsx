import "./translate.css";

interface Props {
  segments: { source: string; target: string }[];
}

export default function BilingualView({ segments }: Props) {
  return (
    <div className="bilingual-view">
      {segments.map((seg, i) => (
        <div key={i} className="bilingual-view__row">
          <div className="bilingual-view__source">{seg.source}</div>
          <div className="bilingual-view__target">{seg.target}</div>
        </div>
      ))}
    </div>
  );
}
