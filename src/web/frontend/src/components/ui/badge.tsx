import type { HTMLAttributes } from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center gap-1 rounded-full border px-2.5 py-1 text-[11px] font-semibold uppercase tracking-[0.18em]",
  {
    variants: {
      variant: {
        default: "border-border bg-secondary/60 text-secondary-foreground",
        success: "border-emerald-400/30 bg-emerald-400/10 text-foreground",
        warning: "border-amber-400/30 bg-amber-400/10 text-foreground",
        destructive: "border-rose-400/30 bg-rose-400/10 text-foreground",
        info: "border-sky-400/30 bg-sky-400/10 text-foreground",
        secondary: "border-border bg-secondary/60 text-secondary-foreground",
        outline: "border-border/70 bg-transparent text-muted-foreground",
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

