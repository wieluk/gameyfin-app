import { IconButton } from "@/components/ui/Button";
import { backend } from "@/lib/backend";
import { helpUrl, type HelpTopic } from "@/lib/help";

/** Opens the docs at the section about what is on screen. */
export function HelpButton({ topic, size = "md" }: { topic: HelpTopic; size?: "sm" | "md" }) {
  return (
    <IconButton
      icon="help"
      label="Help"
      size={size}
      onClick={() => void backend.openUrl(helpUrl(topic))}
    />
  );
}
