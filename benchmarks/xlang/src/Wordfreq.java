import java.util.HashMap;
public class Wordfreq {
    public static void main(String[] a) {
        final int n = 3000000;
        long t0 = System.nanoTime();
        HashMap<String, Long> m = new HashMap<>();
        long seed = 42;
        for (int i = 0; i < n; i++) {
            seed = seed * 48271 % 2147483647L;
            m.merge("w" + (seed % 50000), 1L, Long::sum);
        }
        long maxf = 0;
        for (long v : m.values()) if (v > maxf) maxf = v;
        System.out.println("CHECK " + (m.size() * 1000000L + maxf));
        System.out.println("MS " + (System.nanoTime() - t0) / 1000000);
    }
}
