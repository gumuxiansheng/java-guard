public class TriggerJ001J008 {
    public void bad() {
        System.out.println("x"); // J001: no_system_out (YAML)
        try {
            int x = 1;
        } catch (Exception e) {
            // J008: empty catch (builtin)
        }
    }
}
