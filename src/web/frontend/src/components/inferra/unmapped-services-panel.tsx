import { useMemo, useState } from "react";
import { Link } from "react-router-dom";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";

type UnmappedServicesPanelProps = {
  services: string[];
  topN?: number;
};

export function UnmappedServicesPanel({ services, topN = 12 }: UnmappedServicesPanelProps) {
  const [query, setQuery] = useState("");
  const [showAll, setShowAll] = useState(false);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const rows = needle ? services.filter((service) => service.toLowerCase().includes(needle)) : services;
    return rows.sort((left, right) => right.localeCompare(left));
  }, [query, services]);

  const grouped = useMemo(() => {
    const buckets = new Map<string, string[]>();
    for (const service of filtered) {
      const prefix = service.includes("_") ? service.split("_")[0] : service.slice(0, 1).toLowerCase();
      const key = prefix || "other";
      const bucket = buckets.get(key) ?? [];
      bucket.push(service);
      buckets.set(key, bucket);
    }
    return [...buckets.entries()].sort((left, right) => right[1].length - left[1].length);
  }, [filtered]);

  const visible = showAll ? filtered : filtered.slice(0, topN);

  if (!services.length) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Unmapped services</CardTitle>
          <CardDescription>Every observed service is linked to a workspace project.</CardDescription>
        </CardHeader>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle className="text-base">Unmapped services</CardTitle>
            <CardDescription>
              {services.length} services have evidence but no workspace owner. Grouped by prefix; map the noisy ones first.
            </CardDescription>
          </div>
          <Badge variant="outline">{services.length} total</Badge>
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        <Input
          aria-label="Filter unmapped services"
          placeholder="Filter by service id…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />

        <div className="flex flex-wrap gap-2">
          {grouped.slice(0, 6).map(([prefix, rows]) => (
            <Badge key={prefix} variant="secondary">
              {prefix} · {rows.length}
            </Badge>
          ))}
        </div>

        <div className="flex flex-wrap gap-2">
          {visible.map((service) => (
            <Button key={service} variant="outline" size="sm" asChild>
              <Link to={`/systems/${encodeURIComponent(service)}`}>{service}</Link>
            </Button>
          ))}
        </div>

        {filtered.length > topN ? (
          <Button variant="ghost" size="sm" onClick={() => setShowAll((value) => !value)}>
            {showAll ? "Show fewer" : `Show all ${filtered.length}`}
          </Button>
        ) : null}
      </CardContent>
    </Card>
  );
}
