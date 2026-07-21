import { baseName, parentPath } from "../api";
import { FileIcon } from "../icons";

export function SearchResults({
  query,
  results,
  onOpen,
}: {
  query: string;
  results: string[];
  onOpen: (p: string) => void;
}) {
  return (
    <div className="search-results">
      <div className="search-header">
        <span>
          {results.length} {results.length === 1 ? "result" : "results"} for “{query}”
        </span>
      </div>
      <ul>
        {results.map((p) => (
          <li key={p} onClick={() => onOpen(p)}>
            <span className="result-icon">
              <FileIcon entry={{ name: baseName(p), is_dir: false, size: 0, mtime: 0 }} />
            </span>
            <span className="result-name">{baseName(p)}</span>
            <span className="result-path">{parentPath(p) || "Vault"}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
