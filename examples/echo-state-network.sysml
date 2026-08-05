package EchoStateNetworks {
    doc /* An Echo State Network: a recurrent network whose input and
         * recurrent weights are drawn at random once and then left alone,
         * so that training collapses to a linear fit of the readout. What
         * the model has to say, therefore, is less about learning than
         * about the reservoir: the hyper-parameters that decide whether it
         * forgets its own initial state, the signals that pass between the
         * three layers, and the order of the phases in one session.
         */

    private import ScalarValues::*;

    // -----------------------------------------------------------------
    // The values that pass between the layers
    // -----------------------------------------------------------------

    attribute def Vector {
        doc /* A real vector, `dimension` components long, in index order. */
        attribute dimension : Positive;
        attribute component : Real[1..*] ordered;
    }

    attribute def Matrix {
        doc /* A real matrix, its elements in row-major order. Every weight
             * matrix here is drawn once from a random distribution; only
             * the readout's is ever fitted. */
        attribute rows : Positive;
        attribute columns : Positive;
        attribute element : Real[1..*] ordered;
    }

    item def Sample {
        doc /* One time step of the driving signal. The teacher output is
             * present while training and absent while running, which is the
             * only difference between the two phases as far as the network
             * is concerned. */
        attribute step : Natural;
        attribute input : Vector;
        attribute teacher : Vector[0..1];
    }

    port def SignalPort {
        doc /* One layer's output, seen from the layer that produces it; the
             * consumer declares the conjugate `~SignalPort`. */
        out attribute signal : Vector;
    }

    // -----------------------------------------------------------------
    // What the layers compute
    // -----------------------------------------------------------------

    abstract calc def Tanh {
        doc /* The reservoir's activation, applied component-wise. The
             * standard function library stops at the circular trigonometric
             * functions, so the model declares this one and leaves the body
             * to the implementation. */
        in x : Vector;
        return : Vector;
    }

    abstract calc def SpectralRadius {
        doc /* The largest absolute eigenvalue of a square matrix. The
             * reservoir is rescaled by this quantity at construction, which
             * is the one moment its weights are touched. */
        in weights : Matrix;
        return : Real;
    }

    calc def EffectiveSpectralRadius {
        doc /* A leaky-integrator reservoir does not contract at the rate of
             * its weight matrix: at the origin its Jacobian is
             * (1 - leak) I + leak W, so the radius that matters is this
             * blend of the two. With leak = 1 it is the weight matrix's own
             * radius, and the classical condition returns. */
        in radius : Real;
        in leak : Real;
        return : Real = (1.0 - leak) + leak * radius;
    }

    calc def ReadoutWidth {
        doc /* The readout reads a bias, the current input and the reservoir
             * state at once, so its design matrix is this wide. */
        in inputDimension : Positive;
        in reservoirSize : Positive;
        return : Positive = 1 + inputDimension + reservoirSize;
    }

    abstract calc def LeakyUpdate {
        doc /* The state update, one time step:
             *
             *   x(t+1) = (1 - a) x(t) + a tanh(W_in [1; u(t+1)] + W x(t))
             *
             * with a the leak rate. `excitation` is the argument of the
             * tanh, already summed by the input layer and the reservoir. */
        in previous : Vector;
        in excitation : Vector;
        in leak : Real;
        return next : Vector;
    }

    abstract calc def ReadoutMap {
        doc /* y(t) = W_out [1; u(t); x(t)] -- the whole of what the trained
             * network computes. */
        in weights : Matrix;
        in input : Vector;
        in reservoirState : Vector;
        return output : Vector;
    }

    constraint def EchoStateProperty {
        doc /* The echo state property: the reservoir's dependence on its
             * initial state dies out, so its memory is of the driving
             * signal alone. Contraction of the update map is what forces
             * that, and an effective spectral radius below one is the usual
             * sufficient condition -- necessary in practice, not in theory,
             * since a strongly driven reservoir can hold together above
             * one. */
        in radius : Real;
        in leak : Real;
        EffectiveSpectralRadius(radius, leak) < 1.0 and 0.0 < leak and leak <= 1.0
    }

    // -----------------------------------------------------------------
    // The three layers
    // -----------------------------------------------------------------

    part def InputLayer {
        doc /* W_in, drawn once and never trained: it scatters the driving
             * signal across the reservoir, scaled to set how hard the input
             * drives it against its own recurrence. */
        attribute inputDimension : Positive;
        attribute reservoirSize : Positive;
        attribute inputScaling : Real = 1.0;

        attribute weights : Matrix {
            :>> rows = reservoirSize;
            :>> columns = 1 + inputDimension;
        }

        port drive : ~SignalPort;
        port projection : SignalPort;
    }

    part def Reservoir {
        doc /* The fixed random recurrent pool -- the network's memory. Its
             * weights are drawn sparse, then rescaled to `spectralRadius`;
             * after that neither training nor running changes them, and
             * only the state moves. */
        attribute size : Positive = 500;
        attribute spectralRadius : Real = 0.9;
        attribute sparsity : Real = 0.1;
        attribute leakRate : Real = 1.0;
        attribute seed : Natural = 0;

        attribute weights : Matrix {
            :>> rows = size;
            :>> columns = size;
        }

        attribute stateVector : Vector {
            :>> dimension = size;
        }

        port projection : ~SignalPort;
        port activation : SignalPort;

        calc update : LeakyUpdate {
            in previous = stateVector;
            in leak = leakRate;
        }

        assert constraint echoState : EchoStateProperty {
            in radius = spectralRadius;
            in leak = leakRate;
        }
    }

    part def Readout {
        doc /* W_out -- the only trained thing in the network. Fitted in one
             * shot by ridge regression, which is why an ESN trains in a
             * single pass over the data and has no gradient to speak of. */
        attribute inputDimension : Positive;
        attribute reservoirSize : Positive;
        attribute outputDimension : Positive;
        attribute regularization : Real = 0.00000001;
        attribute trained : Boolean = false;

        attribute weights : Matrix {
            :>> rows = outputDimension;
            :>> columns = ReadoutWidth(inputDimension, reservoirSize);
        }

        port activation : ~SignalPort;
        port prediction : SignalPort;
    }

    part def EchoStateNetwork {
        doc /* The three layers wired in one line: the driving signal enters
             * the input layer, its projection excites the reservoir, and
             * the readout reads the reservoir's state. */
        attribute inputDimension : Positive = 1;
        attribute outputDimension : Positive = 1;

        port drive : ~SignalPort;
        port prediction : SignalPort;

        part input : InputLayer {
            :>> inputDimension = EchoStateNetwork::inputDimension;
            :>> reservoirSize = reservoir.size;
        }

        part reservoir : Reservoir;

        part readout : Readout {
            :>> inputDimension = EchoStateNetwork::inputDimension;
            :>> reservoirSize = reservoir.size;
            :>> outputDimension = EchoStateNetwork::outputDimension;
        }

        connect drive to input.drive;
        connect input.projection to reservoir.projection;
        connect reservoir.activation to readout.activation;
        connect readout.prediction to prediction;

        perform action train : TrainingRun {
            in regularization = readout.regularization;
        }

        perform action run : Predict;

        state session : SessionLifecycle;
    }

    // -----------------------------------------------------------------
    // Training and running
    // -----------------------------------------------------------------

    action def RunStep {
        doc /* One time step of the driven reservoir, training or not. */
        in sample : Sample;
        in previous : Vector;
        out nextState : Vector;
    }

    action def Washout {
        doc /* The opening steps, run and thrown away: until the initial
             * state has washed out, the harvested states carry an arbitrary
             * starting point that the readout would happily fit. */
        in stream : Sample[1..*] ordered;
        in steps : Natural;
        out washedState : Vector;

        action step : RunStep[1..*] ordered;
    }

    action def Harvest {
        doc /* Drive the reservoir over the training stream and keep every
             * state it passes through: these rows, not the network, are
             * what the fit sees. */
        in stream : Sample[1..*] ordered;
        in initial : Vector;
        out harvested : Vector[1..*] ordered;
        out teacher : Vector[1..*] ordered;

        action step : RunStep[1..*] ordered;
    }

    action def FitReadout {
        doc /* Ridge regression, in closed form:
             *
             *   W_out = Y X^T (X X^T + b I)^-1
             *
             * The ridge term b is what keeps the fit off the reservoir's
             * own noise, and it matters more here than the reservoir size
             * does. */
        in harvested : Vector[1..*] ordered;
        in teacher : Vector[1..*] ordered;
        in regularization : Real;
        out weights : Matrix;
    }

    action def Predict {
        doc /* Run the trained network forward over an unlabelled stream. */
        in stream : Sample[1..*] ordered;
        in initial : Vector;
        out prediction : Vector[1..*] ordered;

        action step : RunStep[1..*] ordered;
    }

    action def TrainingRun {
        doc /* One session, end to end: wash out, harvest, fit. */
        in stream : Sample[1..*] ordered;
        in regularization : Real;
        out weights : Matrix;

        action wash : Washout {
            in stream = TrainingRun::stream;
        }

        action collect : Harvest {
            in stream = TrainingRun::stream;
        }

        action fit : FitReadout {
            in regularization = TrainingRun::regularization;
        }

        first wash then collect;
        flow from wash.washedState to collect.initial;

        first collect then fit;
        flow from collect.harvested to fit.harvested;
        flow from collect.teacher to fit.teacher;
    }

    state def SessionLifecycle {
        doc /* The phases of one session. The network is untrained until a
             * fit has happened, and nothing but a fit moves it on -- there
             * is no incremental learning to fall back on. */
        attribute washout : Natural = 100;

        entry;
        then untrained;

        state untrained;
        state washingOut;
        state harvesting;
        state fitting;
        state running;

        transition start first untrained accept sample : Sample then washingOut;

        transition washed first washingOut accept sample : Sample if sample.step >= washout then harvesting;

        transition streamEnded first harvesting then fitting;

        transition fitted first fitting then running;

        transition refit first running then washingOut;
    }

    // -----------------------------------------------------------------
    // What is being asked of the network
    // -----------------------------------------------------------------

    requirement def EchoStateRequirement {
        doc /* Without this the network is not an ESN at all: its output
             * would depend on where the reservoir happened to start. */
        subject network : EchoStateNetwork;
        require constraint echoState : EchoStateProperty {
            in radius = network.reservoir.spectralRadius;
            in leak = network.reservoir.leakRate;
        }
    }

    requirement def SingleShotTrainingRequirement {
        doc /* Training shall touch the readout only. The reservoir's
             * weights are what makes the method cheap; refitting them would
             * make it an ordinary recurrent network. */
        subject network : EchoStateNetwork;
    }

    satisfy EchoStateRequirement by EchoStateNetwork;
    satisfy SingleShotTrainingRequirement by EchoStateNetwork;

    analysis def MemoryCapacityAnalysis {
        doc /* How far back the readout can still reconstruct the input:
             * the summed squared correlation over delays. It is bounded by
             * the reservoir size, and a reservoir tuned for memory is not
             * the same one that is tuned for nonlinearity -- the trade-off
             * this analysis exists to expose. */
        subject network : EchoStateNetwork;
        in attribute maxDelay : Positive = 200;
        return capacity : Real;

        objective memoryObjective {
            doc /* Determine the delay beyond which the readout can no
                 * longer recover the driving signal. */
        }
    }

    // -----------------------------------------------------------------
    // A configured network
    // -----------------------------------------------------------------

    part mackeyGlassPredictor : EchoStateNetwork {
        doc /* One-step-ahead prediction of the Mackey-Glass series at
             * tau = 17 -- the benchmark ESNs were introduced on, and still
             * the shortest demonstration that a fixed random reservoir with
             * a linear readout beats a trained recurrent network on chaotic
             * time series. */
        :>> inputDimension = 1;
        :>> outputDimension = 1;

        part :>> reservoir {
            :>> size = 1000;
            :>> spectralRadius = 0.95;
            :>> sparsity = 0.01;
            :>> leakRate = 1.0;
            :>> seed = 42;
        }

        part :>> readout {
            :>> regularization = 0.00000001;
        }

        state :>> session {
            :>> washout = 500;
        }
    }

    comment about EchoStateNetwork /*
     * The root of this model: everything else either flows through it,
     * constrains it or configures it.
     */
}
