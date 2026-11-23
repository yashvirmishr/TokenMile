import type { PricingRegistryView } from '../types';

interface PricingPanelProps {
  registry: PricingRegistryView | null;
  loading: boolean;
  error: string | null;
  onReload: () => void;
  onClose: () => void;
}

/**
 * Pricing maintenance: which models have a price, and which usage has none yet.
 *
 * An unpriced model is listed here with its tokens instead of being folded into
 * a total as `$0` — the point of this panel is to make that gap fixable.
 */
export function PricingPanel({ registry, loading, error, onReload, onClose }: PricingPanelProps) {
  return (
    <section className="panel-open reveal">
      <div className="panel-head">
        <span className="label">Pricing</span>
        <span className="flex items-center gap-1.5">
          {registry && (
            <span className="mono text-[10px] text-bone-3">
              {registry.registry_models.toLocaleString('en-US')} models
            </span>
          )}
          <button onClick={onReload} className="ghost-btn" title="Reload the registry">
            reload
          </button>
          <button onClick={onClose} className="ghost-btn" title="Close [p]">
            close esc
          </button>
        </span>
      </div>

      <div className="max-h-[340px] overflow-y-auto px-4 pb-3">
        {loading && !registry && <p className="caption pt-4">Loading the price registry…</p>}

        {error && (
          <p className="pt-4 text-[11px] text-alert">Pricing registry error: {error}</p>
        )}

        {registry && (
          <>
            <div className="flex items-baseline justify-between pt-3">
              <span className="label">Your models</span>
              <span className="mono text-[9.5px] text-bone-3">
                {registry.version}
                {registry.registry_version ? ` · ${registry.registry_version}` : ''}
              </span>
            </div>

            {registry.known.length === 0 ? (
              <p className="caption py-2">
                None of the models in your history have a price yet.
              </p>
            ) : (
              registry.known.map((p) => (
                <div
                  key={`${p.provider}/${p.model_id}`}
                  className="border-b border-hair/60 py-2 last:border-b-0"
                >
                  <div className="flex items-baseline gap-2">
                    <span className="label">{p.provider}</span>
                    <span className="mono truncate text-[11.5px] text-bone">{p.model_id}</span>
                    <span className={`label ml-auto ${p.is_free ? 'text-verd' : 'text-brass'}`}>
                      {p.is_free ? 'free' : 'paid'}
                    </span>
                  </div>

                  <div className="mono mt-1 flex flex-wrap gap-x-3 text-[10px] text-bone-3">
                    <span>
                      in <span className="text-bone-2">${p.actual.input_per_million.toFixed(2)}</span>/M
                    </span>
                    <span>
                      out <span className="text-bone-2">${p.actual.output_per_million.toFixed(2)}</span>/M
                    </span>
                    {p.actual.cached_per_million > 0 && (
                      <span>
                        cache{' '}
                        <span className="text-bone-2">
                          ${p.actual.cached_per_million.toFixed(4)}
                        </span>
                        /M
                      </span>
                    )}
                    <span className="ml-auto">
                      {p.source === 'ftm_override' ? 'curated' : 'registry'}
                    </span>
                  </div>

                  {p.reference_model && p.reference_model !== p.model_id && (
                    <div className="mono mt-0.5 text-[9.5px] text-bone-3">
                      reference: <span className="text-verd">{p.reference_model}</span>
                    </div>
                  )}
                </div>
              ))
            )}

            <div className="pt-4">
              <span className="label">Unpriced models</span>
              {registry.unknown.length === 0 ? (
                <p className="caption py-2">Every model in your history has a price.</p>
              ) : (
                registry.unknown.map((m) => (
                  <div
                    key={`${m.provider}/${m.model}`}
                    className="border-b border-hair/60 py-2 last:border-b-0"
                  >
                    <div className="flex items-baseline gap-2">
                      <span className="label">{m.provider}</span>
                      <span className="mono truncate text-[11.5px] text-bone">{m.model}</span>
                      <span className="mono ml-auto text-[11px] text-needle">
                        {m.tokens.toLocaleString('en-US')}
                      </span>
                    </div>
                    <div className="mono mt-0.5 text-[9.5px] text-bone-3">
                      {m.records} record(s) · no price entry · excluded from value, never $0
                    </div>
                  </div>
                ))
              )}
            </div>

            <p className="caption pt-4">
              in / out / cache are prices per million tokens. Models missing from the
              registry are reported as unpriced rather than assumed free.
            </p>
          </>
        )}
      </div>
    </section>
  );
}
