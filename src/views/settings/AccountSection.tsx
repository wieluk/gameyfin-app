import { Alert } from "@/components/Alert";
import { Button } from "@/components/ui";
import { backend } from "@/lib/backend";
import { useStatus } from "@/lib/queries";
import { useAction } from "@/lib/useAction";
import { Row, Section } from "./controls";
import { DeviceNameField } from "./saves";

export function AccountSection({ onSignedOut }: { onSignedOut: () => void }) {
  const status = useStatus();
  const signOut = useAction();

  // "Offline" (server unreachable, session still valid) is distinct from "Disconnected"
  // (signed out), so the user is not told they have been logged out when they have not.
  const connection = status.data?.offline
    ? { value: "Offline, showing your cached library", tone: "bad" as const }
    : status.data?.authenticated
      ? { value: "Connected", tone: "good" as const }
      : { value: "Disconnected", tone: "bad" as const };

  return (
    <Section title="Account">
      <Row label="Server" value={status.data?.serverUrl ?? "Not configured"} />
      <Row label="Signed in as" value={status.data?.username ?? "Not signed in"} />
      <Row label="Status" value={connection.value} tone={connection.tone} />
      <div className="pt-2">
        <DeviceNameField />
      </div>
      <div className="pt-1">
        <Button
          variant="destructive"
          disabled={signOut.busy}
          onClick={() =>
            void signOut.run(async () => {
              await backend.signOut();
              onSignedOut();
            })
          }
        >
          Sign out
        </Button>
      </div>
      {signOut.error && <Alert>{signOut.error}</Alert>}
    </Section>
  );
}
