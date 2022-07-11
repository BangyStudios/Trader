enum Currency{BTC};
enum Exchange{Coinbase};

public class Trader {
    private String dataFile;
    private int threshold;

    /**
     * Constructor for Trader
     * @param dataFile Path to dataFile for testing different values
     * @param threshold Percentage threshold
     */
    public Trader(String dataFile, int threshold) {
        this.dataFile = dataFile;
    }

    public static void main(String[] args) {

    }
}
