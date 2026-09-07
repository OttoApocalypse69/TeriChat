-- Milestone E hardening: last-owner invariant as a database backstop.
-- Forward-only: never edit after it has run anywhere; supersede with a new file.
--
-- The application checks ownership first (nice 400 errors), but check-then-act
-- races between concurrent managers. This trigger is the backstop: any UPDATE
-- of `role` or DELETE that would leave a workspace with zero owners aborts
-- the whole transaction. The application maps that abort to a 500 — reachable
-- only through a genuine race, never through the normal guarded path.

CREATE OR REPLACE FUNCTION enforce_workspace_owner()
RETURNS TRIGGER AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM workspace_members
        WHERE workspace_id = COALESCE(NEW.workspace_id, OLD.workspace_id)
          AND role = 'owner'
    ) THEN
        RAISE EXCEPTION 'workspace must keep at least one owner';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER workspace_members_owner_guard
    AFTER UPDATE OF role OR DELETE ON workspace_members
    FOR EACH ROW
    EXECUTE FUNCTION enforce_workspace_owner();
