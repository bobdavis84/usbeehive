import type { CSSProperties } from "react";

export type DiagnosticStatus = "Healthy" | "Warning" | "Critical";
export type DiagnosticConfidence = "High" | "Medium" | "Low";
export type DiagnosticRootCause =
  | "Cable"
  | "Charger"
  | "Sink"
  | "Port"
  | "Firmware"
  | "None";
export type DiagnosticBottleneck =
  | "NoCharger"
  | "ChargerLimit"
  | "CableLimit"
  | "CableNoEMarker"
  | "DeviceLimit"
  | "SinkLimit"
  | "Fine"
  | "ContractOutOfSpec";

/** Mirrors the nullable serde `charging_diag` in mock-data-dbus.json. */
export interface ChargingDiagnostic {
  bottleneck: DiagnosticBottleneck;
  summary: string;
  detail: string;
  is_warning: boolean;
  status: DiagnosticStatus;
  root_cause: DiagnosticRootCause;
  confidence: DiagnosticConfidence;
  recommended_action: string;
  rule_id: string;
}

/** The exact top-level fields this presentational card consumes. */
export interface DeviceCardData {
  headline: string;
  subtitle: string;
  charging_diag: ChargingDiagnostic | null;
}

export interface DeviceCardProps {
  device: DeviceCardData;
}

const diagnosticFields: readonly (keyof ChargingDiagnostic)[] = [
  "bottleneck",
  "summary",
  "detail",
  "is_warning",
  "status",
  "root_cause",
  "confidence",
  "recommended_action",
  "rule_id",
];

/** Render one D-Bus SnapshotJson summary; this component performs no I/O. */
export default function DeviceCard({ device }: DeviceCardProps) {
  const diagnostic = device.charging_diag;
  const style = {
    "--device-card-accent": diagnostic?.is_warning ? "#b54708" : "#2563eb",
  } as CSSProperties;

  return (
    <article
      className="device-card"
      aria-label={`Device: ${device.headline}`}
      data-warning={diagnostic ? String(diagnostic.is_warning) : "false"}
      data-status={diagnostic?.status ?? "Unavailable"}
      style={style}
    >
      <header className="device-card__header">
        <h2 className="device-card__headline">{device.headline}</h2>
        <p className="device-card__subtitle">{device.subtitle}</p>
      </header>

      <section className="device-card__diagnostic" aria-label="charging_diag">
        <h3>charging_diag</h3>
        {diagnostic === null ? (
          <dl className="device-card__fields">
            <div className="device-card__field">
              <dt>charging_diag</dt>
              <dd>null</dd>
            </div>
          </dl>
        ) : (
          <dl className="device-card__fields">
            {diagnosticFields.map((field) => (
              <div className="device-card__field" key={field}>
                <dt>{field}</dt>
                <dd>{String(diagnostic[field])}</dd>
              </div>
            ))}
          </dl>
        )}
      </section>
    </article>
  );
}
