import type { HTMLAttributes } from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center gap-1 rounded-sm border px-1.5 py-0.5 font-data text-[11px] font-medium leading-none",
  {
    variants: {
      variant: {
        default: "border-border bg-panel-inset text-foreground",
        success: "border-success/40 bg-success/10 text-foreground",
        warning: "border-warning/40 bg-warning/10 text-foreground",
        destructive: "border-critical/40 bg-critical/10 text-foreground",
        info: "border-border bg-secondary text-foreground",
        secondary: "border-border bg-panel-inset text-muted-foreground",
        outline: "border-border bg-transparent text-muted-foreground",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

export interface BadgeProps extends HTMLAttributes<HTMLDivElement>, VariantProps<typeof badgeVariants> {}

export function Badge({ className, variant, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant }), className)} {...props} />;
}
