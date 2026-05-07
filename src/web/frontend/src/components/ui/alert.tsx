import type { HTMLAttributes } from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const alertVariants = cva(
  "flex items-start gap-3 rounded-2xl border px-4 py-3 text-sm shadow-sm [&>svg]:mt-0.5 [&>svg]:size-4 [&>svg]:shrink-0",
  {
  variants: {
    variant: {
      default: "border-border bg-card/80 text-foreground",
      warning: "border-amber-400/35 bg-amber-400/12 text-foreground",
      destructive: "border-rose-400/35 bg-rose-400/12 text-foreground",
      success: "border-emerald-400/35 bg-emerald-400/12 text-foreground",
      info: "border-sky-400/35 bg-sky-400/12 text-foreground",
    },
  },
  defaultVariants: {
    variant: "default",
  },
});

export interface AlertProps extends HTMLAttributes<HTMLDivElement>, VariantProps<typeof alertVariants> {}

export function Alert({ className, variant, ...props }: AlertProps) {
  return <div className={cn(alertVariants({ variant }), className)} role="alert" {...props} />;
}

export function AlertTitle({ className, ...props }: HTMLAttributes<HTMLParagraphElement>) {
  return <p className={cn("font-semibold", className)} {...props} />;
}

export function AlertDescription({ className, ...props }: HTMLAttributes<HTMLParagraphElement>) {
  return <p className={cn("mt-1 leading-6 text-current/90", className)} {...props} />;
}

