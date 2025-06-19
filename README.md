# minecraft-pdb-mgr
minecraft-pgb-mgr watches a Minecraft server for connected players and updates a
Kubernetes PodDisruptionBudget to prevent server shutdowns when players are
connected.

# How?
Deploy minecraft-pdb-mgr to your cluster (only one replica is needed) and set
the following environment variables:

- `POD_NAMESPACE` - the namespace the pod runs in. You should use
`valueFrom.fieldRef.fieldPath: metadata.namespace` for this.
- `RUST_LOG` (optional) - the log level (i.e. `info`, `debug,`, `warn`, `error`)
- `UPDATE_INTERVAL` - how often in seconds to check for players and to patch the
  PDB. The default value is `10`.
- `PDB_NAME` - the name of the PBB object in the same namespace as the pod to
update. You need to create this PDB yourself.
- `SERVER_HOST` - the hostname or IP address of the Minecraft server to monitor.
- `SERVER_PORT` - the port of the Minecraft server to monitor.
- `MIN_PLAYERS` - the minimum number of online players to consider when updating
the PDB. Default is 1.
- `MIN_PLAYERS_PERCENT` - a floating point value (`0.0` - `1.0`) representing
the percent of online players to the maximum number of players to consider when
updating the PDB. This takes precedence over `MIN_PLAYERS`.

# License

See [LICENSE.md](LICENSE.md).
